//! Thread-local generation-stamped scratch arena.
//!
//! Every pathfinding, BFS, LOS, connectivity, and AI spatial query reuses the same
//! thread-local buffers. Resetting between queries is O(1) (just incrementing a `u32`
//! generation counter), so zero heap allocations occur per query.

use std::cell::RefCell;

pub struct PathScratch {
    pub gen: u32,
    pub num_tiles: usize,

    // General BFS / single-turn / A* arrays (indexed by tile)
    pub seen_gen: Vec<u32>,
    pub closed_gen: Vec<u32>,
    pub todo_gen: Vec<u32>,
    pub todo_dmg: Vec<u8>,

    // A* RouteNode storage
    pub route_nodes: Vec<u64>,
    pub route_parent: Vec<i32>,

    // Single-turn classic movement arrays
    pub dist_f32: Vec<f32>,
    pub parent_i32: Vec<i32>,
    pub reached_list: Vec<i32>,
    pub wave_curr: Vec<i32>,
    pub wave_next: Vec<i32>,

    // Priority queue (min-heap of packed u64) and flat BFS queue
    pub heap: Vec<u64>,
    pub queue: Vec<i32>,

    // Vision LOS arrays
    pub los_gen: u32,
    pub los_ring_tag: Vec<u32>,
    pub los_max_height: Vec<i8>,
    pub los_ring_curr: Vec<i32>,
    pub los_ring_next: Vec<i32>,
    pub los_out: Vec<i32>,

    // Scratch bitset / marks for secondary operations (vision deltas, multi-target sets)
    pub mark_gen: u32,
    pub mark: Vec<u32>,
    pub mark_aux: Vec<i32>,
}

impl PathScratch {
    fn new() -> Self {
        Self {
            gen: 1,
            num_tiles: 0,
            seen_gen: Vec::new(),
            closed_gen: Vec::new(),
            todo_gen: Vec::new(),
            todo_dmg: Vec::new(),
            route_nodes: Vec::new(),
            route_parent: Vec::new(),
            dist_f32: Vec::new(),
            parent_i32: Vec::new(),
            reached_list: Vec::new(),
            wave_curr: Vec::new(),
            wave_next: Vec::new(),
            heap: Vec::new(),
            queue: Vec::new(),
            los_gen: 1,
            los_ring_tag: Vec::new(),
            los_max_height: Vec::new(),
            los_ring_curr: Vec::new(),
            los_ring_next: Vec::new(),
            los_out: Vec::new(),
            mark_gen: 1,
            mark: Vec::new(),
            mark_aux: Vec::new(),
        }
    }

    #[inline]
    pub fn ensure_size(&mut self, n: usize) {
        if self.num_tiles >= n {
            return;
        }
        self.num_tiles = n;
        self.seen_gen.resize(n, 0);
        self.closed_gen.resize(n, 0);
        self.todo_gen.resize(n, 0);
        self.todo_dmg.resize(n, 0);
        self.route_nodes.resize(n, 0);
        self.route_parent.resize(n, -1);
        self.dist_f32.resize(n, 0.0);
        self.parent_i32.resize(n, -1);
        self.los_ring_tag.resize(n, 0);
        self.los_max_height.resize(n, 0);
        self.mark.resize(n, 0);
        self.mark_aux.resize(n, -1);
    }

    /// O(1) generation bump for pathfinding/BFS.
    #[inline]
    pub fn next_gen(&mut self, n: usize) -> u32 {
        self.ensure_size(n);
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            self.seen_gen.fill(0);
            self.closed_gen.fill(0);
            self.todo_gen.fill(0);
            self.gen = 1;
        }
        self.heap.clear();
        self.queue.clear();
        self.reached_list.clear();
        self.wave_curr.clear();
        self.wave_next.clear();
        self.gen
    }

    /// O(1) generation bump for LOS (reserves `rings` consecutive tags).
    #[inline]
    pub fn next_los_base(&mut self, n: usize, rings: u32) -> u32 {
        self.ensure_size(n);
        let needed = rings.saturating_add(4);
        if self.los_gen > u32::MAX - needed {
            self.los_ring_tag.fill(0);
            self.los_gen = 1;
        }
        let base = self.los_gen;
        self.los_gen += needed;
        self.los_ring_curr.clear();
        self.los_ring_next.clear();
        self.los_out.clear();
        base
    }

    /// O(1) generation bump for secondary marker array (can be nested inside a `next_gen` pass).
    #[inline]
    pub fn next_mark(&mut self, n: usize) -> u32 {
        self.ensure_size(n);
        self.mark_gen = self.mark_gen.wrapping_add(1);
        if self.mark_gen == 0 {
            self.mark.fill(0);
            self.mark_gen = 1;
        }
        self.mark_gen
    }

    /// Binary min-heap push on `self.heap`.
    #[inline]
    pub fn heap_push(&mut self, val: u64) {
        self.heap.push(val);
        let mut idx = self.heap.len() - 1;
        while idx > 0 {
            let parent = (idx - 1) >> 1;
            if self.heap[idx] < self.heap[parent] {
                self.heap.swap(idx, parent);
                idx = parent;
            } else {
                break;
            }
        }
    }

    /// Binary min-heap pop on `self.heap`.
    #[inline]
    pub fn heap_pop(&mut self) -> Option<u64> {
        let len = self.heap.len();
        if len == 0 {
            return None;
        }
        let top = self.heap[0];
        let last = self.heap.pop().unwrap();
        let new_len = len - 1;
        if new_len > 0 {
            self.heap[0] = last;
            let mut idx = 0;
            loop {
                let left = (idx << 1) + 1;
                if left >= new_len {
                    break;
                }
                let right = left + 1;
                let smallest = if right < new_len && self.heap[right] < self.heap[left] {
                    right
                } else {
                    left
                };
                if self.heap[smallest] < self.heap[idx] {
                    self.heap.swap(idx, smallest);
                    idx = smallest;
                } else {
                    break;
                }
            }
        }
        Some(top)
    }
}

thread_local! {
    static SCRATCH: RefCell<PathScratch> = RefCell::new(PathScratch::new());
}

#[inline]
pub fn with_scratch<R>(f: impl FnOnce(&mut PathScratch) -> R) -> R {
    SCRATCH.with(|cell| f(&mut cell.borrow_mut()))
}
