//! Prints this machine's meters every 2 s with how long each read took
//! (card #599): `cargo run -p irori-sys --example meters`.

fn main() {
    let t = std::time::Instant::now();
    let mut s = sys::meters::Sampler::new();
    println!("new: {:?}", t.elapsed());
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let t = std::time::Instant::now();
        let x = s.sample();
        println!("sample: {:?}\n{x:?}", t.elapsed());
    }
}
