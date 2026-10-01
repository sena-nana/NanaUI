use std::io::{self, BufRead, Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let map_path = std::env::args()
        .nth(1)
        .ok_or("usage: nana-sfc-remap <map.json>")?;
    let mut map = String::new();
    std::fs::File::open(map_path)?.read_to_string(&mut map)?;
    for line in io::stdin().lock().lines() {
        let line = line?;
        println!("{}", nana_ui_sfc::diagnostics::remap_json_line(&line, &map));
    }
    Ok(())
}
