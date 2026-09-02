use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pdfx", about = "Visually-lossless PDF compressor")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show where the bytes live
    Inspect { input: PathBuf },
    /// Compress; never emit a larger file
    Compress {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        report: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Inspect { input } => {
            let bytes = fs::read(&input).with_context(|| format!("read {}", input.display()))?;
            let report = pdfx_core::inspect_file(&bytes)?;
            println!("pages:    {}", report.pages);
            println!("objects:  {}", report.objects);
            println!("bytes:    {}", report.bytes);
            for n in report.notes {
                println!("{n}");
            }
        }
        Cmd::Compress {
            input,
            output,
            report,
        } => {
            let bytes = fs::read(&input).with_context(|| format!("read {}", input.display()))?;
            let (out, stats) = pdfx_core::compress_file(&bytes)?;
            let dest = output.unwrap_or_else(|| {
                let mut p = input.clone();
                p.set_extension("pdfx.pdf");
                p
            });
            fs::write(&dest, &out).with_context(|| format!("write {}", dest.display()))?;
            if report || stats.kept_original {
                println!(
                    "{} -> {} bytes ({})",
                    stats.input_bytes,
                    stats.output_bytes,
                    if stats.kept_original {
                        "kept original"
                    } else {
                        "compressed"
                    }
                );
                for n in stats.notes {
                    println!("{n}");
                }
            }
        }
    }
    Ok(())
}
