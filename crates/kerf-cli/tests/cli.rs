//! Integration tests pinning the `kerf` exit-code contract
//! (0 = sound, 1 = unsound, 2 = usage/read error, 3 = nothing to verify).

use std::path::PathBuf;
use std::process::Command;

const KERF: &str = env!("CARGO_BIN_EXE_kerf");

fn write_gcode(name: &str, content: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn verify_exit_code(gcode: &str, name: &str) -> Option<i32> {
    let path = write_gcode(name, gcode);
    Command::new(KERF)
        .args(["verify", path.to_str().unwrap()])
        .output()
        .unwrap()
        .status
        .code()
}

#[test]
fn sound_input_exits_0() {
    let g = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG1 X1 Y1 E1\nG1 X2 Y2 E1\n";
    assert_eq!(verify_exit_code(g, "sound.gcode"), Some(0));
}

#[test]
fn no_geometry_exits_3() {
    let g = "M104 S200\nG28 ; home\n;only comments and setup\n";
    assert_eq!(verify_exit_code(g, "empty.gcode"), Some(3));
}

#[test]
fn version_flag_prints_version_and_exits_0() {
    let out = Command::new(KERF).arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("kerf "));
    assert!(s.trim().ends_with(env!("CARGO_PKG_VERSION")));
}

#[test]
fn missing_file_exits_2() {
    let code = Command::new(KERF)
        .args(["verify", "/no/such/file.gcode"])
        .output()
        .unwrap()
        .status
        .code();
    assert_eq!(code, Some(2));
}

#[test]
fn batch_verify_exit_code_reflects_the_worst_file() {
    let good = write_gcode(
        "bg.gcode",
        "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG1 X1 Y1 E1\nG1 X2 Y2 E1\n",
    );
    let empty = write_gcode("be.gcode", "M104 S200\nG28\n");
    let run = |paths: &[&str]| {
        let mut args = vec!["verify"];
        args.extend_from_slice(paths);
        Command::new(KERF)
            .args(args)
            .output()
            .unwrap()
            .status
            .code()
    };
    let g = good.to_str().unwrap();
    let e = empty.to_str().unwrap();
    assert_eq!(run(&[g, g]), Some(0));
    assert_eq!(run(&[g, e]), Some(1));
}

#[test]
fn diff_identical_exits_0_and_different_exits_1() {
    let a = write_gcode(
        "da.gcode",
        "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG1 X0 Y0 E1\nG1 X20 Y0 E1\n",
    );
    let b = write_gcode(
        "db.gcode",
        "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG1 X40 Y40 E1\nG1 X60 Y40 E1\n",
    );
    let run = |x: &std::path::Path, y: &std::path::Path| {
        Command::new(KERF)
            .args(["diff", x.to_str().unwrap(), y.to_str().unwrap()])
            .output()
            .unwrap()
            .status
            .code()
    };
    assert_eq!(run(&a, &a), Some(0));
    assert_eq!(run(&a, &b), Some(1));
}
