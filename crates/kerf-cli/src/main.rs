//! `kerf` CLI: parse slicer G-code into the IR and verify that Kerf's operations preserve the
//! deposited material.

use std::process::ExitCode;

use kerf_core::frontend::{parse, Diagnostics};
use kerf_core::json;
use kerf_core::verify::verify_gcode;

const USAGE: &str = "\
kerf — verify and inspect 3D-printer G-code through the Kerf IR

USAGE:
    kerf verify  <file.gcode>... [--resolution <um>] [--json]
    kerf inspect <file.gcode> [--json]
    kerf diff    <a.gcode> <b.gcode> [--resolution <um>] [--json]

COMMANDS:
    verify    Parse the G-code, then check that a Kerf pass preserves the deposited material and that
              the geometry is translation-invariant. One file prints a full report; several files run
              as a batch (one line each, JSON array with --json). Exits non-zero if ANY file is not
              sound — usable as a CI gate over a directory of prints.
    inspect   Parse the G-code and report what was recovered, guessed, or dropped.
    diff      Compare two files by the material they deposit (matched by layer height). Exits 0 if
              identical, 1 if they differ — a real answer to \"do these two slicers make the same part?\"

OPTIONS:
    --resolution <um>   Raster resolution in microns for the checks (default 200).
    --json              Emit the report as JSON instead of text.
    -V, --version       Print the version.
    -h, --help          Show this help.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("error: {msg}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }

    if args[0] == "-V" || args[0] == "--version" {
        println!("kerf {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }

    let cmd = args[0].clone();
    let mut files: Vec<String> = Vec::new();
    let mut json_out = false;
    let mut resolution = 200i64;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json_out = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "--resolution" => {
                i += 1;
                resolution = args
                    .get(i)
                    .ok_or("--resolution needs a value")?
                    .parse()
                    .map_err(|_| "invalid --resolution (expected microns)")?;
            }
            s if !s.starts_with('-') => files.push(s.to_string()),
            s => return Err(format!("unexpected argument: {s}")),
        }
        i += 1;
    }

    if resolution <= 0 {
        return Err("--resolution must be a positive number of microns".to_string());
    }

    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("cannot read {p}: {e}"));

    match cmd.as_str() {
        "verify" => {
            if files.is_empty() {
                return Err("missing <file.gcode>".to_string());
            }
            if files.len() == 1 {
                cmd_verify(&read(&files[0])?, resolution, json_out)
            } else {
                cmd_verify_batch(&files, resolution, json_out)
            }
        }
        "inspect" => cmd_inspect(
            &read(files.first().ok_or("missing <file.gcode>")?)?,
            json_out,
        ),
        "diff" => {
            if files.len() < 2 {
                return Err("diff needs two files: kerf diff <a.gcode> <b.gcode>".to_string());
            }
            cmd_diff(&read(&files[0])?, &read(&files[1])?, resolution, json_out)
        }
        other => Err(format!("unknown command '{other}' (try `kerf --help`)")),
    }
}

fn cmd_verify(src: &str, resolution_um: i64, json_out: bool) -> Result<ExitCode, String> {
    let v = verify_gcode(src, resolution_um);
    if json_out {
        println!("{}", json::to_json(&v).map_err(|e| e.to_string())?);
    } else {
        print_diagnostics(&v.diagnostics);
        println!();
        println!("verification @ {} µm resolution", v.resolution_um);
        println!(
            "  pass preserves denotation:  {}",
            yesno(v.pass_preserves_denotation)
        );
        println!(
            "  pass preserves deposition:  {}",
            yesno(v.pass_preserves_deposit)
        );
        println!(
            "  translation-invariant:      {}",
            yesno(v.translation_invariant)
        );
        println!();
        let headline = if !v.has_geometry {
            "NOTHING TO VERIFY — no extruding geometry was recovered from this file"
        } else if v.ok() {
            "SOUND — Kerf's operations preserve this print"
        } else {
            "UNSOUND — a check failed (see above)"
        };
        println!("  {headline}");
    }
    // 0 sound, 1 unsound, 3 nothing to verify (2 is usage error).
    Ok(if !v.has_geometry {
        ExitCode::from(3)
    } else if v.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Verify several files as a batch: one status line each (or a JSON array); non-zero exit if any
/// file is not sound.
fn cmd_verify_batch(
    files: &[String],
    resolution_um: i64,
    json_out: bool,
) -> Result<ExitCode, String> {
    let mut reports = Vec::with_capacity(files.len());
    for f in files {
        let src = std::fs::read_to_string(f).map_err(|e| format!("cannot read {f}: {e}"))?;
        reports.push((f.clone(), verify_gcode(&src, resolution_um)));
    }
    let all_ok = reports.iter().all(|(_, v)| v.ok());

    if json_out {
        println!("{}", json::to_json(&reports).map_err(|e| e.to_string())?);
    } else {
        for (f, v) in &reports {
            let status = if !v.has_geometry {
                "NOTHING"
            } else if v.ok() {
                "SOUND"
            } else {
                "UNSOUND"
            };
            println!("{status:8} {f}");
        }
        let sound = reports.iter().filter(|(_, v)| v.ok()).count();
        println!("\n{sound}/{} sound", reports.len());
    }
    Ok(if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn cmd_inspect(src: &str, json_out: bool) -> Result<ExitCode, String> {
    let report = parse(src);
    if json_out {
        println!(
            "{}",
            json::to_json(&report.diagnostics).map_err(|e| e.to_string())?
        );
    } else {
        print_diagnostics(&report.diagnostics);
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_diff(a: &str, b: &str, resolution_um: i64, json_out: bool) -> Result<ExitCode, String> {
    let d = kerf_core::diff_gcode(a, b, resolution_um);
    if json_out {
        println!("{}", json::to_json(&d).map_err(|e| e.to_string())?);
    } else {
        println!("diff @ {} µm resolution", d.resolution_um);
        println!("  layers compared:   {}", d.layers.len());
        println!("  cells only in A:   {}", d.total_only_in_a);
        println!("  cells only in B:   {}", d.total_only_in_b);
        println!("  cells shared:      {}", d.total_shared);
        match d.iou() {
            Some(iou) => println!("  similarity (IoU):  {iou:.4}"),
            None => println!("  similarity (IoU):  n/a (both empty)"),
        }
        println!();
        let headline = if d.both_empty {
            "NOTHING TO COMPARE — no extruding geometry recovered from either file".to_string()
        } else if d.identical {
            format!(
                "IDENTICAL — same deposited material (up to {} µm)",
                d.resolution_um
            )
        } else {
            "DIFFER — deposited material is not the same".to_string()
        };
        println!("  {headline}");
    }
    // 0 identical, 1 differ, 3 nothing to compare.
    Ok(if d.both_empty {
        ExitCode::from(3)
    } else if d.identical {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn print_diagnostics(d: &Diagnostics) {
    println!("parsed {} lines", d.lines);
    println!("  layers:            {}", d.layers);
    println!("  extruding paths:   {}", d.extruding_toolpaths);
    println!("  travel paths:      {}", d.travel_toolpaths);
    println!("  estimated widths:  {} moves", d.estimated_width_moves);
    if !d.unknown_roles.is_empty() {
        println!(
            "  unknown roles:     {:?} across {} moves (filed as Perimeter)",
            d.unknown_roles, d.fallback_role_moves
        );
    }
    if d.zero_length_extrudes > 0 {
        println!("  prime/unretract:   {}", d.zero_length_extrudes);
    }
    if d.arcs_flattened > 0 {
        println!(
            "  arcs flattened:    {} (G2/G3 -> chords)",
            d.arcs_flattened
        );
    }
    if d.skipped_g91_moves > 0 {
        println!("  skipped G91 moves: {}", d.skipped_g91_moves);
    }
    if d.overflow_skips > 0 {
        println!("  overflow skips:    {}", d.overflow_skips);
    }
    if !d.skipped_codes.is_empty() {
        println!("  unhandled codes:   {:?}", d.skipped_codes);
    }
}

fn yesno(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "NO"
    }
}
