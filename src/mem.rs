// Flags for InFlightRefs and version

use std::alloc::{alloc, dealloc, Layout};
use std::ptr;


pub fn allocate<I>(value: I) -> *mut I {
    let layout = Layout::new::<I>();
    let raw_ptr = unsafe { alloc(layout) } as *mut I;
    unsafe {
        ptr::write(raw_ptr, value);
    }
    raw_ptr
}

pub fn deallocate<I>(value: *mut I, dealloc_ptr: bool) {
    let layout = Layout::new::<I>();
    unsafe {
        ptr::drop_in_place(value);

        if dealloc_ptr {
            dealloc(value as *mut u8, layout);
        }
    }
}
