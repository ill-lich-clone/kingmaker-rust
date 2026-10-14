fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("allocate") => {
            let mut chunks = Vec::new();
            loop {
                chunks.push(vec![42u8; 8 * 1024 * 1024]);
                std::hint::black_box(&chunks);
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        }
        Some("sleep") => std::thread::sleep(std::time::Duration::from_secs(120)),
        Some("fail") => std::process::exit(7),
        _ => for a in args { println!("{a}"); },
    }
}
