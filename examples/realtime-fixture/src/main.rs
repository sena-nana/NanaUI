use realtime_fixture::Options;

const USAGE: &str = "usage: realtime-fixture [--evidence] [--bodies N] [--present-hz N]";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut number = |name: &str| -> Result<u32, String> {
            args.next()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| format!("{name} needs a number\n{USAGE}"))
        };
        match arg.as_str() {
            "--evidence" => options.evidence = true,
            // Clamped: the fixture is a light load by default and stays one.
            "--bodies" => options.bodies = number("--bodies")?.clamp(1, 4096) as usize,
            "--present-hz" => options.present_hz = number("--present-hz")?.clamp(1, 480),
            _ => return Err(USAGE.into()),
        }
    }
    if let Some(report) = realtime_fixture::run(options)? {
        print!("{report}");
    }
    Ok(())
}
