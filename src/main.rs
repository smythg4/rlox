use rlox::vm::{InterpretResult, Vm};

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(
    version,
    about = "rlox — a Lox bytecode VM written in Rust",
    long_about = "rlox compiles and executes Lox source files using a single-pass bytecode\n\
                  compiler and a stack-based VM with mark-and-sweep garbage collection.\n\n\
                  If no file is provided, rlox starts an interactive REPL."
)]
struct Cli {
    /// Lox source file to execute. Omit to start the REPL.
    filename: Option<PathBuf>,

    /// Print a full stack and instruction trace on every VM cycle.
    #[arg(short, long)]
    tracing: bool,

    /// Disassemble compiled bytecode before execution.
    #[arg(short, long)]
    debug: bool,

    /// Log GC activity: allocations, mark/sweep cycles, and freed objects.
    #[arg(short, long)]
    gc_logging: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut vm = Vm::new();

    if cli.debug {
        vm = vm.with_debug();
    }
    if cli.tracing {
        vm = vm.with_tracing();
    }
    if cli.gc_logging {
        vm = vm.with_gc_log();
    }

    if let Some(path) = cli.filename {
        run_file(path, &mut vm)?;
    } else {
        repl(&mut vm)?;
    }
    Ok(())
}

pub fn repl(vm: &mut Vm) -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut buf = String::new();
    write!(stdout, "> ")?;
    stdout.flush()?;

    while let Ok(n) = stdin.read_line(&mut buf) {
        if n == 0 {
            break;
        }
        vm.interpret(&buf[0..n]);
        write!(stdout, "> ")?;
        stdout.flush()?;
        buf.clear();
    }
    Ok(())
}

pub fn run_file(path: PathBuf, vm: &mut Vm) -> Result<()> {
    let buf = std::fs::read_to_string(path)?;
    match vm.interpret(&buf) {
        InterpretResult::CompileError => std::process::exit(65),
        InterpretResult::RuntimeError => std::process::exit(70),
        InterpretResult::Ok => {}
    }
    Ok(())
}
