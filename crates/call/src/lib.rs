pub fn _probe() {
    let _ = opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip);
}
