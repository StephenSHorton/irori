//! Prints PATH after the desktop app's fix (sys::shellpath), for checking
//! it under launchd's PATH: `env -i HOME=$HOME SHELL=$SHELL PATH=/usr/bin:/bin`.
fn main() {
    // SAFETY: no other thread yet.
    unsafe { sys::shellpath::fix_gui_path() };
    println!("{}", std::env::var("PATH").unwrap_or_default());
}
