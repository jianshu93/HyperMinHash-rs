use hyperminhash::Sketch;
use rayon::prelude::*;
use std::{
    env,
    error::Error,
    fs::File,
    io::{BufRead, BufReader},
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let p: u32 = arguments
        .next()
        .ok_or("usage: parallel P FILE...")?
        .parse()?;
    let files: Vec<_> = arguments.collect();
    if files.is_empty() {
        return Err("provide at least one input file".into());
    }
    Sketch::with_precision(p)?;
    let sketches: std::io::Result<Vec<Sketch>> = files
        .par_iter()
        .map(|path| {
            let reader = BufReader::new(File::open(path)?);
            let mut sketch = Sketch::with_precision(p).expect("precision was validated");
            for line in reader.lines() {
                sketch.add_bytes(line?.as_bytes());
            }
            Ok(sketch)
        })
        .collect();
    let mut combined = Sketch::with_precision(p)?;
    for sketch in sketches? {
        combined.merge(&sketch)?;
    }
    println!("{:.0}", combined.cardinality());
    Ok(())
}
