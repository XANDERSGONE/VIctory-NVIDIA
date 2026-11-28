//! `VICTORY_BARRIER_TRIM` ([`super::barrier_trim`]): a console wave64 program run as two host
//! waves of 32 gets fewer group barriers, and only where they order nothing. These tests lower
//! and count; they need no host GPU.

use super::{lower, shader};
use crate::native::{
    ComputeIo, HwStage, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource,
};
use crate::{io, ir, resource, structure};

const S_ENDPGM: u32 = 0xBF81_0000;

/// A wave64 compute program of `threads` threads (thread index in v0) with 4 root constants
/// in s0..s3, a raw UAV's descriptor in s[8:11] and, with `lds`, LDS; then `body`.
fn native(body: &[u32], threads: u32, lds: bool) -> NativeProgram {
    let mut words = vec![
        0xBE85_0380, // s_mov_b32 s5, 0
        0x3D << 26 | 0x02 << 18 | 8 << 6 | 2,
        125 << 25, // s_load_dwordx4 s[8:11], s[4:5], 0
    ];
    words.extend_from_slice(body);
    words.push(S_ENDPGM);
    NativeProgram {
        stage: HwStage::Cs,
        wave64: true,
        vgprs: 16,
        layout: vec![
            RootParam {
                root: RootType::Constants,
                table_offset_dwords: 0,
                sgprs: 4,
                base_register: 0,
                user_data: 0,
                slot: 0,
            },
            RootParam {
                root: RootType::Table,
                table_offset_dwords: 0,
                sgprs: 1,
                base_register: 0,
                user_data: 4,
                slot: 1,
            },
        ],
        used: vec![UsedResource {
            kind: ResourceType::Uav,
            root: RootType::Table,
            table_offset_dwords: 0,
            lower_bound: 0,
            stride: 0,
            dwords: 4,
            user_data: 4,
            slot: 1,
        }],
        io: StageIo::Cs(ComputeIo {
            rsrc2: 5 << 1 | if lds { 1 << 15 } else { 0 }, // 5 user SGPRs: s0..s4
            threads: [threads, 1, 1],
        }),
        code: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
    }
}

/// The program lowered for a host of 32-lane waves with the trim on or off: its DXIL and the
/// count of barrier operations emitted while lowering it.
fn lowered(native: &NativeProgram, trim: bool) -> (Vec<u8>, u32) {
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    let st = structure::structure(&p).expect("structure");
    let res =
        resource::track(&p, &native.layout, native.stage, Some(&st)).expect("resource::track");
    let io = io::raise(&p, native).expect("io::raise");
    let wave = lower::TEST_HOST_WAVE.replace(Some(32));
    let was = lower::TEST_BARRIER_TRIM.replace(Some(trim));
    shader::BARRIER_OPS.set(0);
    let l = lower::lower_counted(
        &lower::Parts {
            native,
            p: &p,
            st: &st,
            io: &io,
            res: &res,
            inputs: Vec::new(),
            outputs: Vec::new(),
            prims: Vec::new(),
            mesh_stride: 0,
        },
        false,
    );
    let n = shader::BARRIER_OPS.get();
    lower::TEST_BARRIER_TRIM.set(was);
    lower::TEST_HOST_WAVE.set(wave);
    (l.expect("lower").dxil, n)
}

/// Two wave ORs (each ends in `v_readlane`, so the halves exchange through LDS) and a store.
fn two_wave_ors() -> Vec<u32> {
    let mut w = vec![0x3402_0082]; // v_lshlrev_b32 v1, 2, v0 (only to have v1 set)
    w.push(0x7E0E_0300); // v_mov_b32 v7, v0
    w.extend_from_slice(&super::wave::WAVE_OR_V7_TO_S7);
    w.push(0x7E10_0207); // v_mov_b32 v8, s7
    w.push(0x4A0E_0107); // v_add_nc_u32 v7, v7, v0
    w.extend_from_slice(&super::wave::WAVE_OR_V7_TO_S7);
    w.extend_from_slice(&[
        0x4A00_1007, // v_add_nc_u32 v0, s7, v8
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
    ]);
    w
}

/// Exchanges in one block take the two pairs of exchange dwords in turn: the barrier after
/// each exchange's reads is the next exchange's first. Two in a block end with pair 1 owed,
/// which the next block may leave so (its first exchange writes pair 0): no barrier at the end.
#[test]
fn back_to_back_exchanges_share_their_barriers() {
    let native = native(&two_wave_ors(), 64, false);
    let (off_dxil, off) = lowered(&native, false);
    let (on_dxil, on) = lowered(&native, true);
    assert_ne!(off_dxil, on_dxil, "the trim changes the program");
    assert!(off >= 4, "two exchanges, two barriers each: {off}");
    assert_eq!(
        off - on,
        2,
        "the second barrier of each exchange: {on} of {off}"
    );
}

/// One exchange in a block leaves pair 0 owed at its end: the barrier after its reads stays,
/// at the end, so the next block finds pair 0 free.
#[test]
fn a_lone_exchange_keeps_its_two_barriers() {
    let mut w = vec![0x7E0E_0300]; // v_mov_b32 v7, v0
    w.extend_from_slice(&super::wave::WAVE_OR_V7_TO_S7);
    w.extend_from_slice(&[
        0x7E00_0207, // v_mov_b32 v0, s7
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
    ]);
    let native = native(&w, 64, false);
    let (_, off) = lowered(&native, false);
    let (_, on) = lowered(&native, true);
    assert_eq!(on, off);
}

/// LDS reads one after another need no meeting of the halves between them; a write still
/// gets one before it, and the first read after a write too.
#[test]
fn lds_reads_after_a_meeting_need_no_meeting_of_their_own() {
    let mut words = vec![
        0x3402_0082, // v_lshlrev_b32 v1, 2, v0
        0xD834_0000,
        0x0000_0001, // ds_write_b32 v1, v0
        0xD8D8_0000,
        0x0400_0001, // ds_read_b32 v4, v1
        0xD8D8_0000,
        0x0500_0001, // ds_read_b32 v5, v1
        0xD8D8_0000,
        0x0600_0001, // ds_read_b32 v6, v1
        0xBF8C_C07F, // s_waitcnt lgkmcnt(0)
        0x4A0E_0B04, // v_add_nc_u32 v7, v4, v5
        0x4A0E_0D07, // v_add_nc_u32 v7, v7, v6
    ];
    // (a wave OR: the program reads across its halves, so it runs as two host waves with an
    // LDS meeting before each access)
    words.extend_from_slice(&super::wave::WAVE_OR_V7_TO_S7);
    words.extend_from_slice(&[
        0x7E00_0207, // v_mov_b32 v0, s7
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
    ]);
    let native = native(&words, 128, true);
    let (_, off) = lowered(&native, false);
    let (_, on) = lowered(&native, true);
    // off: a meeting before each of the 4 accesses, and the exchange's 2. On: before the
    // write and the first read, and the exchange's first: the meeting before it freed both
    // pairs, so it takes pair 1 and the block ends with only that one owed.
    assert_eq!(
        off - on,
        3,
        "the two reads after the first need none: {on} of {off}"
    );
}

/// With the trim off every access gets its meeting, the same each time.
#[test]
fn without_the_trim_every_access_gets_its_meeting() {
    let words = [
        0x3402_0082, // v_lshlrev_b32 v1, 2, v0
        0xD8D8_0000,
        0x0400_0001, // ds_read_b32 v4, v1
        0xD8D8_0000,
        0x0500_0001, // ds_read_b32 v5, v1
    ];
    let mut w = words.to_vec();
    w.push(0x4A0E_0B04); // v_add_nc_u32 v7, v4, v5
    w.extend_from_slice(&super::wave::WAVE_OR_V7_TO_S7);
    w.extend_from_slice(&[
        0x7E00_0207, // v_mov_b32 v0, s7
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
    ]);
    let native = native(&w, 128, true);
    let (a, n1) = lowered(&native, false);
    let (b, n2) = lowered(&native, false);
    assert_eq!((a, n1), (b, n2), "the same each time");
    let (_, on) = lowered(&native, true);
    // the second read needs no meeting, and the exchange after the first's takes pair 1
    assert_eq!(n1 - on, 2, "{on} of {n1}");
}
