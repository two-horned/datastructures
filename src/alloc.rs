use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::{self, NonNull};
use std::alloc::{alloc, dealloc};
use std::collections::BinaryHeap;
use std::sync::Mutex;

impl PoolAlloc {
    pub fn new(layout: Layout, capacity: usize) -> Self {
        let (memlyt, objsiz) = layout.repeat(capacity).unwrap();
        let memory = unsafe { alloc(memlyt) };

        Self {
            objsiz,
            objalg: layout.align(),
            frptrs: Mutex::new(
                (0..capacity)
                    .map(|x| unsafe { memory.add(x * objsiz) })
                    .collect(),
            ),
            memlyt,
            memory,
        }
    }
}

unsafe impl Sync for PoolAlloc {}

impl Drop for PoolAlloc {
    fn drop(&mut self) {
        unsafe { dealloc(self.memory, self.memlyt) }
    }
}

pub struct PoolAlloc {
    objsiz: usize,
    objalg: usize,
    frptrs: Mutex<Vec<*mut u8>>,
    memlyt: Layout,
    memory: *mut u8,
}

unsafe impl Allocator for PoolAlloc {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.align() > self.objalg || layout.size() > self.objsiz {
            return Err(AllocError);
        }
        let mut frptrs = self.frptrs.lock().unwrap();
        if let Some(start) = frptrs.pop() {
            let ptr = ptr::from_raw_parts_mut(start, layout.size());
            return Ok(unsafe { NonNull::new_unchecked(ptr) });
        }
        Err(AllocError)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _: Layout) {
        let mut frptrs = self.frptrs.lock().unwrap();
        frptrs.push(ptr.as_ptr());
    }
}

impl StacklikeAlloc {
    pub fn new(layout: Layout) -> Self {
        let memlyt = layout.pad_to_align();

        Self {
            contrl: Mutex::new(StacklikeCtrl::new()),
            memlyt,
            memory: unsafe { alloc(memlyt) },
        }
    }
}

unsafe impl Sync for StacklikeAlloc {}

impl Drop for StacklikeAlloc {
    fn drop(&mut self) {
        unsafe { dealloc(self.memory, self.memlyt) }
    }
}

unsafe impl Allocator for StacklikeAlloc {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.align() > self.memlyt.align() {
            return Err(AllocError);
        }

        let mut contrl = self.contrl.lock().unwrap();
        let mask = layout.align() - 1;
        let start = contrl.offset + mask & !mask;
        let end = start + layout.size();

        if end > self.memlyt.size() {
            return Err(AllocError);
        }

        let ptr = ptr::from_raw_parts_mut(unsafe { self.memory.add(start) }, layout.size());
        let offset = contrl.offset;
        contrl.alptrs.push(offset);
        contrl.offset = end;
        Ok(unsafe { NonNull::new_unchecked(ptr) })
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let mut contrl = self.contrl.lock().unwrap();

        let offset = unsafe { ptr.as_ptr().offset_from_unsigned(self.memory) + layout.size() };
        contrl.frptrs.push(offset);

        while let Some(&x) = contrl.frptrs.peek() {
            assert!(x <= contrl.offset);
            if x < contrl.offset {
                break;
            }

            contrl.offset = contrl.alptrs.pop().unwrap();
            contrl.frptrs.pop();
        }
    }
}

pub struct StacklikeAlloc {
    contrl: Mutex<StacklikeCtrl>,
    memlyt: Layout,
    memory: *mut u8,
}

impl StacklikeCtrl {
    fn new() -> Self {
        Self {
            offset: 0,
            alptrs: Vec::new(),
            frptrs: BinaryHeap::new(),
        }
    }
}

pub struct StacklikeCtrl {
    offset: usize,
    alptrs: Vec<usize>,
    frptrs: BinaryHeap<usize>,
}
