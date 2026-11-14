fn main() {
    let path = std::env::args().nth(1).expect("plugin path");
    let bytes = std::fs::read(path).unwrap();
    let image = moan_plugin::elf::Image::parse(&bytes).expect("admitted freestanding plugin");
    println!(
        "image={} segments={} clap_entry={:#x}",
        image.size,
        image.segments.len(),
        image
            .export(
                std::env::args()
                    .nth(2)
                    .as_deref()
                    .unwrap_or("clap_entry")
                    .as_bytes()
            )
            .unwrap()
    );
}
