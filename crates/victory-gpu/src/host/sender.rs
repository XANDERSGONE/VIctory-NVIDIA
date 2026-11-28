//! Submissions sent ahead are given to the card by a thread of their own,
//! `victory-send` (`VICTORY_SEND_THREAD=1` or `2`; off by default), so that
//! the thread that prepares them goes on to the next one as soon as this one
//! is prepared.
//!
//! Why: the thread that prepares submissions is the frame's wall on every PC
//! measured (48 of 55 ms on an i9-12900K, `docs/findings.md`; 51 of 53 ms on
//! an i9-13900KF with an RTX 4080, upstream PR 475). Its refreshes and
//! snapshots cannot be split from the next submission's: a host copy has one
//! CPU side and one snapshot. What comes after them can: the calls that give
//! the lists to the queue, and the lists of earlier flights made ready again
//! (level 1); also the wait for the round of the heaps' records begun for
//! the submission, and the making of the slots that waited for a texture's
//! host copy (level 2). The frame-rate line of the submitting thread names
//! what that was ("sending the lists", "waiting for the heaps' records",
//! "after the sending"); this one names what the sender took.
//!
//! What keeps it in order:
//!
//! - One thread, first in first out: the card is given what the queue was
//!   given before, in the same order, with every fence value higher than the
//!   one before ([`give`]). The lists of copies in the card's memory that go
//!   ahead of a submission (`Host::card_before`) go through it too.
//! - Only who holds the submission lock gives it lists to send (the
//!   mapping of tiles, and the landing of flights, which reaches no queue,
//!   are given by others: below, and [`land`]). A submission is in
//!   the line of flights (`line()`) before the lock is let go, as before, so
//!   whatever the next submission looks at (what flights may write, which of
//!   them it waits for) sees it; a wait for a flight not given to the queue
//!   yet ends when the sender has given it and the card has run it.
//! - Every other call that puts work on the host's queue waits until the
//!   sender has nothing in hand ([`drain`]): a submission that is waited for,
//!   the fetch of what shaders wrote that the submission lock waits for, and
//!   the swap chain's present, which shows what the queue was given. The one
//!   that cannot wait, the mapping of a reserved texture's tiles (made while
//!   lists are recorded and while textures are refreshed, under the texture's
//!   lock, which the round of the heaps' records may want), is given to the
//!   sender instead ([`tiles`]): it reaches the queue behind what was given
//!   before it, as it reached it behind what was sent before it.
//! - The sender takes no lock of the submitting thread's and waits for no
//!   fence: a thread that waits for a flight never waits for it in turn.
//! - At most [`RUNS_MOST`] submissions are given and not sent: the thread
//!   that prepares waits beyond that.
//!
//! Level 2 takes the textures given host copies so far when the submission
//! is given, not when the round of its heaps' records is over: a texture made
//! by that round itself has its slots made before the next submission, so
//! it shows a submission later than at level 1, never earlier. (Taking them
//! when the round is over would take some of the next submission's too.)
//!
//! Off where the order of the queue says more than this: without the relay's
//! second stage and its fetches over the copy queue (a fetch on the drawing
//! queue is numbered against the sends to copies in the card's memory; where
//! no copy queue could be made, such a fetch empties the sender with that
//! numbering held), with
//! a second queue planned (`VICTORY_ASYNC_QUEUE`), and with the present not
//! sent ahead.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::*;

/// Submissions given and not sent, at most.
const RUNS_MOST: usize = 2;

/// `VICTORY_SEND_THREAD`: 0 (off), 1 (the queue's calls and landed lists),
/// 2 (also the wait for the heaps' records and the slots that waited).
fn wanted() -> u8 {
    static LEVEL: OnceLock<u8> = OnceLock::new();
    *LEVEL.get_or_init(|| match std::env::var("VICTORY_SEND_THREAD").ok().as_deref().map(str::trim) {
        Some("1") => 1,
        Some("2") => 2,
        _ => 0,
    })
}

/// The level in force: [`wanted`] where the rest allows it and the thread
/// runs, else 0.
pub(super) fn level() -> u8 {
    static LEVEL: OnceLock<u8> = OnceLock::new();
    *LEVEL.get_or_init(|| {
        let level = wanted();
        if level == 0 {
            return 0;
        }
        if !relay_exact() || !relay_copy() || !late_free_present() || two_queues::on() {
            tracing::warn!(
                "VICTORY_SEND_THREAD needs VICTORY_RELAY=2 with VICTORY_RELAY_COPY=1 and no VICTORY_ASYNC_QUEUE: \
                 submissions are sent by the thread that prepares them"
            );
            return 0;
        }
        if !started() {
            return 0;
        }
        tracing::info!(
            level,
            most = RUNS_MOST,
            "submissions sent ahead are given to the card by a thread of their own (VICTORY_SEND_THREAD={level})"
        );
        level
    })
}

/// Whether the sender is on.
pub(super) fn on() -> bool {
    level() > 0
}

/// A submission sent ahead, as the thread that prepared it leaves it.
pub(super) struct Run {
    /// Closed lists, in the order they go to the queue.
    pub(super) lists: Vec<Option<ID3D12CommandList>>,
    /// Where another call to the queue begins (`joins`).
    pub(super) cuts: Vec<usize>,
    /// The fence value signalled behind them.
    pub(super) value: u64,
    /// Level 2: the round of the heaps' records to wait for, and the
    /// textures whose waiting slots are made after it.
    pub(super) heaps: Option<(crate::descriptor::SyncedHeaps, Vec<usize>)>,
    /// For the timeline: its stamp, and whether the title's queue is a
    /// compute one.
    #[cfg(feature = "diagnostics")]
    pub(super) stamp: Option<usize>,
    #[cfg(feature = "diagnostics")]
    pub(super) compute: bool,
}

/// What the thread is given.
enum Item {
    /// Lists that go to the queue as they are, with no signal: the copies in
    /// the card's memory ahead of a submission.
    Lists(Vec<Option<ID3D12CommandList>>),
    Run(Run),
    /// Tiles of a reserved texture mapped to a heap ([`tiles`]).
    Tiles(Tiles),
    /// The lists of flights that have landed, reset and kept ([`land`]).
    Land,
}

/// One region of a reserved texture's tiles, mapped to ranges of a heap
/// (`ID3D12CommandQueue::UpdateTileMappings`, one region, `flags`, `starts`
/// and `counts` one per range).
pub(crate) struct Tiles {
    pub(crate) texture: ID3D12Resource,
    pub(crate) at: D3D12_TILED_RESOURCE_COORDINATE,
    pub(crate) size: D3D12_TILE_REGION_SIZE,
    pub(crate) heap: ID3D12Heap,
    pub(crate) flags: Vec<D3D12_TILE_RANGE_FLAGS>,
    pub(crate) starts: Vec<u32>,
    pub(crate) counts: Vec<u32>,
}

impl Tiles {
    /// Maps them on `queue`.
    pub(crate) fn map(&self, queue: &ID3D12CommandQueue) {
        // SAFETY: a live reserved texture and a live heap; one region;
        // `flags.len()` ranges in three arrays of that length, alive through
        // the call.
        unsafe {
            queue.UpdateTileMappings(
                &self.texture,
                1,
                Some(&raw const self.at),
                Some(&raw const self.size),
                &self.heap,
                self.flags.len() as u32,
                Some(self.flags.as_ptr()),
                Some(self.starts.as_ptr()),
                Some(self.counts.as_ptr()),
                D3D12_TILE_MAPPING_FLAG_NONE,
            );
        }
    }
}

// SAFETY: closed command lists, a fence value, plain numbers, and a
// texture and a heap held for a mapping. The queue's calls are
// free-threaded, and a list given here is touched by no other thread until
// its flight has landed (as when the thread that prepared it sent it).
unsafe impl Send for Item {}

impl Item {
    /// Whether it is a submission.
    fn runs(&self) -> bool {
        matches!(self, Item::Run(_))
    }

    /// Whether it puts work on the queue.
    fn queued(&self) -> bool {
        !matches!(self, Item::Land)
    }
}

/// A line of work for one thread: first in first out, with room for
/// `most` items that count ([`Line::give`]), and a wait for what goes to the
/// queue ([`Line::drain`]).
struct Line<T> {
    state: Mutex<State<T>>,
    moved: Condvar,
}

struct State<T> {
    items: VecDeque<T>,
    /// Given and not done (in the line or in hand), that go to the queue.
    unsent: usize,
    /// Of those, the ones that count against the room.
    runs: usize,
}

impl<T> Line<T> {
    const fn new() -> Self {
        Line {
            state: Mutex::new(State {
                items: VecDeque::new(),
                unsent: 0,
                runs: 0,
            }),
            moved: Condvar::new(),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Puts `item` at the end. One that `runs` waits while `most` such are
    /// given and not done. Returns how long it waited.
    fn give(&self, item: T, runs: bool, queued: bool, most: usize) -> Duration {
        let began = Instant::now();
        let mut state = self.state();
        while runs && state.runs >= most {
            state = self.moved.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        let waited = began.elapsed();
        state.runs += usize::from(runs);
        state.unsent += usize::from(queued);
        state.items.push_back(item);
        drop(state);
        self.moved.notify_all();
        waited
    }

    /// Takes the oldest item, waiting for one. It stays counted until
    /// [`Line::done`].
    fn take(&self) -> T {
        let mut state = self.state();
        loop {
            if let Some(item) = state.items.pop_front() {
                return item;
            }
            state = self.moved.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// The item taken last is done.
    fn done(&self, runs: bool, queued: bool) {
        let mut state = self.state();
        state.runs -= usize::from(runs);
        state.unsent -= usize::from(queued);
        drop(state);
        self.moved.notify_all();
    }

    /// Waits until everything given that goes to the queue is done.
    /// Whether it had to wait.
    fn drain(&self) -> bool {
        let mut state = self.state();
        let waits = state.unsent > 0;
        while state.unsent > 0 {
            state = self.moved.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        waits
    }
}

static LINE: Line<Item> = Line::new();

thread_local! {
    /// This is the sender.
    static OURS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Since the last frame-rate line: submissions sent, lists of copies sent,
/// microseconds sending, waiting for the heaps' records (level 2), landing
/// lists; on the threads that give and drain: microseconds waited for
/// room, drains that waited, microseconds of those.
static STATS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
/// The fence value of the last submission sent ([`Host::send_run`]).
static LAST: AtomicU64 = AtomicU64::new(0);
/// Submissions that reached the sender with a value not above the last.
static OUT_OF_ORDER: AtomicU64 = AtomicU64::new(0);
const SENT: usize = 0;
const LISTS: usize = 1;
const SENDING: usize = 2;
const HEAPS: usize = 3;
const LANDING: usize = 4;
const ROOM: usize = 5;
const DRAINS: usize = 6;
const DRAINED: usize = 7;

fn give(item: Item) {
    let (runs, queued) = (item.runs(), item.queued());
    let waited = LINE.give(item, runs, queued, RUNS_MOST);
    STATS[ROOM].fetch_add(waited.as_micros() as u64, Relaxed);
}

/// A submission prepared under the submission lock, for the sender to give
/// to the queue.
pub(super) fn run(run: Run) {
    give(Item::Run(run));
}

/// Lists that go to the queue as they are, in their place among the sends.
pub(super) fn lists(lists: Vec<Option<ID3D12CommandList>>) {
    give(Item::Lists(lists));
}

/// The lists of flights that have landed are reset and kept by the sender.
pub(super) fn land() {
    give(Item::Land);
}

/// `tiles` to be mapped on `queue`: given to the sender if `queue` is the
/// host's and the sender is on (then they reach the queue in their place
/// among what it was given), else given back to be mapped at once.
pub(super) fn tiles(queue: &ID3D12CommandQueue, tiles: Tiles) -> Option<Tiles> {
    if !on() || OURS.get() || !host().is_some_and(|h| h.queue.as_raw() == queue.as_raw()) {
        return Some(tiles);
    }
    give(Item::Tiles(tiles));
    None
}

/// Before a call that puts work on the host's queue by itself: whatever the
/// sender was given is on the queue. (Nothing on the sender itself, and
/// nothing with the sender off.)
pub(super) fn drain() {
    if OURS.get() || !on() {
        return;
    }
    let began = Instant::now();
    if LINE.drain() {
        STATS[DRAINS].fetch_add(1, Relaxed);
        STATS[DRAINED].fetch_add(began.elapsed().as_micros() as u64, Relaxed);
    }
}

/// Starts the thread, once: whether there is one.
fn started() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let worker = || {
            use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL};
            // SAFETY: this thread's own priority.
            unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) };
            crate::cores::keep_on_fast_cores("victory-send");
            OURS.set(true);
            let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(serve));
            if ended.is_err() {
                // Nothing would go to the card again, and the thread that
                // prepares would stand at its next wait for good.
                fail(format_args!("the thread that gives submissions to the card panicked"));
            }
        };
        match std::thread::Builder::new().name("victory-send".into()).spawn(worker) {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(error = %e, "no thread to give submissions to the card: the thread that prepares them does");
                false
            }
        }
    })
}

fn serve() {
    let Some(host) = host() else {
        fail(format_args!("the thread that gives submissions to the card found no host device"));
    };
    loop {
        let item = LINE.take();
        let (runs, queued) = (item.runs(), item.queued());
        let began = Instant::now();
        match item {
            Item::Lists(lists) => {
                // SAFETY: closed lists of ours, live until their flight lands.
                unsafe { host.queue.ExecuteCommandLists(&lists) };
                STATS[LISTS].fetch_add(1, Relaxed);
                STATS[SENDING].fetch_add(began.elapsed().as_micros() as u64, Relaxed);
            }
            Item::Run(run) => host.send_run(run),
            Item::Tiles(tiles) => {
                tiles.map(&host.queue);
                crate::host::tiles_mapped_on(&host.queue);
            }
            Item::Land => {
                let _landing = crate::timeline::span("sender: landed lists reset and given back");
                super::land();
                STATS[LANDING].fetch_add(began.elapsed().as_micros() as u64, Relaxed);
            }
        }
        LINE.done(runs, queued);
    }
}

impl Host {
    /// What the thread that prepared `run` did after its preparing, as it
    /// did it (`Host::submit`), but for the flight's noting.
    fn send_run(&self, run: Run) {
        let Run {
            lists,
            cuts,
            value,
            heaps,
            #[cfg(feature = "diagnostics")]
            stamp,
            #[cfg(feature = "diagnostics")]
            compute,
        } = run;
        if let Some((synced, made)) = heaps {
            let _waiting = crate::timeline::span("sender: waiting for the heaps' records");
            let began = Instant::now();
            drop(synced);
            if crate::heap_marks::on() {
                crate::heap_marks::heaps_waited(began.elapsed().as_nanos() as u64);
            }
            // The slots that waited for a texture's host copy, before the
            // lists that may read them run.
            crate::descriptor::make_waiting_of(made);
            STATS[HEAPS].fetch_add(began.elapsed().as_micros() as u64, Relaxed);
        }
        // Each value above the last one sent: given in the order they were
        // taken, under the submission lock. Said if not, and sent anyway (held
        // back, its flight would never land). After the same check in
        // Kemerd's send thread (upstream PR 18).
        let last = LAST.swap(value, Relaxed);
        if value <= last && OUT_OF_ORDER.fetch_add(1, Relaxed) < 4 {
            tracing::warn!(value, last, "a submission reached the sender out of order (VICTORY_SEND_THREAD)");
        }
        let _sending = crate::timeline::span("sender: sending");
        let began = Instant::now();
        #[cfg(feature = "diagnostics")]
        let sending = crate::timeline::ticks();
        // SAFETY: closed lists; the fence is ours, and its values come here
        // in the order they were taken.
        let signalled = unsafe {
            let mut from = 0;
            for to in cuts.into_iter().chain([lists.len()]) {
                self.queue.ExecuteCommandLists(&lists[from..to]);
                from = to;
            }
            self.queue.Signal(&self.fence, value)
        };
        awake::sent(&self.device);
        #[cfg(feature = "diagnostics")]
        crate::timeline::sent(sending, value, stamp, compute);
        self.card_sent();
        if let Err(e) = signalled {
            fail(format_args!("a submission sent ahead could not be followed by a signal ({e})"));
        }
        STATS[SENT].fetch_add(1, Relaxed);
        STATS[SENDING].fetch_add(began.elapsed().as_micros() as u64, Relaxed);
    }
}

/// The frame-rate line's part: what the sender did per frame.
pub(super) fn line(frames: f64) {
    if !on() {
        return;
    }
    let [sent, lists, sending, heaps, landing, room, drains, drained] =
        std::array::from_fn::<f64, 8, _>(|i| STATS[i].swap(0, Relaxed) as f64 / frames);
    tracing::info!(
        "the sender thread per frame (VICTORY_SEND_THREAD={}): {sent:.1} submissions and {lists:.1} lists of copies sent \
         in {:.1} ms, {:.1} ms waiting for the heaps' records, {:.1} ms landed lists reset and given back; the threads \
         that give it work waited {:.1} ms for room, {drains:.1} drains {:.1} ms",
        level(),
        sending / 1e3,
        heaps / 1e3,
        landing / 1e3,
        room / 1e3,
        drained / 1e3,
    );
}

#[cfg(test)]
mod tests {
    use super::Line;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Items come out in the order they went in, whatever the room.
    #[test]
    fn items_come_out_in_order() {
        let line: Arc<Line<(u64, bool)>> = Arc::new(Line::new());
        let out = Arc::new(std::sync::Mutex::new(Vec::new()));
        let worker = {
            let (line, out) = (line.clone(), out.clone());
            std::thread::spawn(move || {
                loop {
                    let (n, runs) = line.take();
                    out.lock().unwrap().push(n);
                    line.done(runs, true);
                    if n == 999 {
                        break;
                    }
                }
            })
        };
        for n in 0..1000u64 {
            let runs = n % 3 != 0;
            line.give((n, runs), runs, true, 2);
        }
        worker.join().unwrap();
        assert_eq!(*out.lock().unwrap(), (0..1000).collect::<Vec<u64>>());
    }

    /// No more than `most` items that count are given and not done.
    #[test]
    fn the_room_is_kept() {
        let line: Arc<Line<bool>> = Arc::new(Line::new());
        let held = Arc::new(AtomicUsize::new(0));
        let most_seen = Arc::new(AtomicUsize::new(0));
        let worker = {
            let (line, held, most_seen) = (line.clone(), held.clone(), most_seen.clone());
            std::thread::spawn(move || {
                for _ in 0..500 {
                    let runs = line.take();
                    most_seen.fetch_max(held.load(Ordering::SeqCst), Ordering::SeqCst);
                    std::thread::yield_now();
                    if runs {
                        held.fetch_sub(1, Ordering::SeqCst);
                    }
                    line.done(runs, true);
                }
            })
        };
        for n in 0..500 {
            let runs = n % 4 != 3;
            // Counted before it is given: the worker may take it at once.
            if runs {
                held.fetch_add(1, Ordering::SeqCst);
            }
            line.give(runs, runs, true, 2);
        }
        worker.join().unwrap();
        // One more than the room: the giver counts before its wait.
        assert!(most_seen.load(Ordering::SeqCst) <= 3, "{}", most_seen.load(Ordering::SeqCst));
    }

    /// A drain returns only once everything that goes to the queue is done,
    /// and does not wait for what does not.
    #[test]
    fn a_drain_waits_for_what_goes_to_the_queue() {
        let line: Arc<Line<(bool, u64)>> = Arc::new(Line::new());
        let done = Arc::new(AtomicUsize::new(0));
        line.give((true, 1), true, true, 2);
        line.give((false, 2), false, true, 2);
        line.give((false, 3), false, false, 2);
        let worker = {
            let (line, done) = (line.clone(), done.clone());
            std::thread::spawn(move || {
                for _ in 0..3 {
                    let (runs, n) = line.take();
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    let queued = n != 3;
                    if queued {
                        done.fetch_add(1, Ordering::SeqCst);
                    }
                    line.done(runs, queued);
                    if n == 2 {
                        // The last one does not go to the queue: a drain
                        // may return before it is done.
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            })
        };
        assert!(line.drain());
        assert_eq!(done.load(Ordering::SeqCst), 2);
        worker.join().unwrap();
        assert!(!line.drain());
    }
}
