// Include!()'d by every fuzz_targets/*.rs bin — a bin's own compilation units
// participate in the link whole, so this definition is always present.
//
// libFuzzer's FuzzerInterceptors.cpp wraps bcmp/memcmp/strcmp/strncmp/
// strcasecmp/strncasecmp/strstr/strcasestr/memmem and forwards each call
// through a `real_<fn>` pointer that its bootstrap resolves with dlsym(). In
// a statically linked musl binary dlsym is a stub that always fails, so
// every `real_*` stays NULL and the first dictionary mutation jumps through
// it (reproduced deterministically at seed 42: Mutate_AddWordFromTORC →
// MakeDictionaryEntryFromCMP → memmem → 0x0).
//
// Defining dlsym here intercepts that bootstrap: the interceptor's lookups
// resolve to the implementations below whenever it lazily initializes, so
// its wrappers forward to real code instead of arming a crash. Nothing else
// in a static fuzz binary calls dlsym; unknown names yield NULL, exactly
// what the static stub would have returned.
use std::ffi::{c_char, c_int, c_void, CStr};

fn bytes_or_empty(pointer: *const u8, len: usize) -> &'static [u8] {
    // SAFETY: a NULL or empty range has no readable bytes to expose; a
    // non-NULL pointer with `len` is the interceptor's own calling contract.
    unsafe {
        if len == 0 || pointer.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts(pointer, len)
        }
    }
}

fn compare(left: &[u8], right: &[u8], case_insensitive: bool, bounded: bool) -> c_int {
    let limit = if bounded {
        left.len().min(right.len())
    } else {
        left.len().max(right.len())
    };
    for index in 0..limit {
        let mut a = *left.get(index).unwrap_or(&0);
        let mut b = *right.get(index).unwrap_or(&0);
        if case_insensitive {
            a = a.to_ascii_lowercase();
            b = b.to_ascii_lowercase();
        }
        if a != b {
            return if a < b { -1 } else { 1 };
        }
        if !bounded && (a == 0 || b == 0) {
            break;
        }
    }
    0
}

fn cstr(bytes: *const c_char) -> Option<&'static [u8]> {
    if bytes.is_null() {
        return None;
    }
    // SAFETY: a non-NULL C string pointer is the calling contract; CStr
    // stops at the first NUL exactly like the libc functions it substitutes.
    Some(unsafe { CStr::from_ptr(bytes) }.to_bytes())
}

fn substring(haystack: &[u8], needle: &[u8], case_insensitive: bool) -> *mut c_void {
    if needle.is_empty() {
        return haystack.as_ptr() as *mut c_void;
    }
    let matches = |window: &[u8]| {
        if case_insensitive {
            window.eq_ignore_ascii_case(needle)
        } else {
            window == needle
        }
    };
    haystack
        .windows(needle.len())
        .position(matches)
        .map_or(std::ptr::null_mut(), |at| haystack[at..].as_ptr() as *mut c_void)
}

fn memmem_impl(haystack: *const c_void, haystack_len: usize, needle: *const c_void, needle_len: usize) -> *mut c_void {
    if needle_len == 0 {
        return haystack as *mut c_void;
    }
    if needle_len > haystack_len {
        return std::ptr::null_mut();
    }
    substring(
        bytes_or_empty(haystack as *const u8, haystack_len),
        bytes_or_empty(needle as *const u8, needle_len),
        false,
    )
}


#[unsafe(no_mangle)]
pub extern "C" fn dlsym(_handle: *mut c_void, name: *const c_char) -> *mut c_void {
    let looked_up = cstr(name);
    let Some(name) = looked_up else {
        return std::ptr::null_mut();
    };
    match name {
        b"bcmp" | b"memcmp" => memicmp_impl as *mut c_void,
        b"strcmp" => strcmp_impl as *mut c_void,
        b"strncmp" => strncmp_impl as *mut c_void,
        b"strcasecmp" => strcasecmp_impl as *mut c_void,
        b"strncasecmp" => strncasecmp_impl as *mut c_void,
        b"strstr" => strstr_impl as *mut c_void,
        b"strcasestr" => strcasestr_impl as *mut c_void,
        b"memmem" => memmem_impl as *mut c_void,
        _ => std::ptr::null_mut(),
    }
}

// The same static-musl weakness leaves libFuzzer's weakly-looked-up crash
// reporting hooks unresolved (its startup prints "Failed to find function
// __sanitizer_…" for each). The timeout handler calls through the null
// PrintStackTrace and itself dies as a "deadly signal", hiding the real
// timeout report. Providing them keeps reporting honest; there is no
// unwinder in these binaries, so the stack-trace hooks are no-ops.
#[unsafe(no_mangle)]
extern "C" fn __sanitizer_acquire_crash_state() -> bool {
    true
}

#[unsafe(no_mangle)]
extern "C" fn __sanitizer_print_stack_trace() {}

#[unsafe(no_mangle)]
extern "C" fn __sanitizer_set_death_callback(_callback: Option<extern "C" fn()>) {}

extern "C" fn memicmp_impl(left: *const c_void, right: *const c_void, len: usize) -> c_int {
    compare(
        bytes_or_empty(left as *const u8, len),
        bytes_or_empty(right as *const u8, len),
        false,
        true,
    )
}

extern "C" fn strcmp_impl(left: *const c_char, right: *const c_char) -> c_int {
    match (cstr(left), cstr(right)) {
        (Some(left), Some(right)) => compare(left, right, false, false),
        _ => 0,
    }
}

extern "C" fn strncmp_impl(left: *const c_char, right: *const c_char, len: usize) -> c_int {
    match (cstr(left), cstr(right)) {
        (Some(left), Some(right)) => compare(&left[..len.min(left.len())], &right[..len.min(right.len())], false, true),
        _ => 0,
    }
}

extern "C" fn strcasecmp_impl(left: *const c_char, right: *const c_char) -> c_int {
    match (cstr(left), cstr(right)) {
        (Some(left), Some(right)) => compare(left, right, true, false),
        _ => 0,
    }
}

extern "C" fn strncasecmp_impl(left: *const c_char, right: *const c_char, len: usize) -> c_int {
    match (cstr(left), cstr(right)) {
        (Some(left), Some(right)) => compare(&left[..len.min(left.len())], &right[..len.min(right.len())], true, true),
        _ => 0,
    }
}

extern "C" fn strstr_impl(haystack: *const c_char, needle: *const c_char) -> *mut c_char {
    match (cstr(haystack), cstr(needle)) {
        (Some(haystack), Some(needle)) => substring(haystack, needle, false) as *mut c_char,
        _ => std::ptr::null_mut(),
    }
}

extern "C" fn strcasestr_impl(haystack: *const c_char, needle: *const c_char) -> *mut c_char {
    match (cstr(haystack), cstr(needle)) {
        (Some(haystack), Some(needle)) => substring(haystack, needle, true) as *mut c_char,
        _ => std::ptr::null_mut(),
    }
}
