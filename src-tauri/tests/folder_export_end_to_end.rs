// Proves the real path, not the stubbed-bootstrap unit tests in export/mod.rs: builds the actual
// lowarc-bootstrap.exe from the sibling `lowarc` checkout, stages a real export folder around it,
// then actually RUNS the exported executable and confirms the module inside it really executed.
//
// #[ignore]d by default — unlike every other test in this repo, this one depends on a checkout
// that isn't (and shouldn't be) vendored into this one: the sibling `lowarc` repo next to this
// one on disk (see export::bootstrap_source's own header for why that's a known, temporary
// bridge). Run explicitly with `cargo test --test folder_export_end_to_end -- --ignored` on a
// machine that actually has both repos checked out side by side.

use lowarc_studio_lib::export::{self, bootstrap_source, ExportOptions};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_folder_export_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore]
fn a_folder_export_actually_runs_its_module_when_launched() {
    if !cfg!(windows) {
        return; // fixture is a PowerShell script, same scoping as runtime_end_to_end.rs
    }

    let modules_dir = temp_dir("modules");
    let module_dir = modules_dir.join("echo");
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::write(module_dir.join("manifest.json"), r#"{"id":"echo","name":"Echo","loadOrder":1,"requires":[]}"#).unwrap();
    std::fs::write(
        module_dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true}"#,
    )
    .unwrap();
    std::fs::write(
        module_dir.join("module.ps1"),
        r#"
$marker = Join-Path $PSScriptRoot "..\..\ran.txt"
while ($line = [Console]::In.ReadLine()) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $msg = $line | ConvertFrom-Json
    switch ($msg.phase) {
        "compile" { $reply = @{ ok = $true } }
        "start" { Set-Content -Path $marker -Value "ran"; $reply = @{ ok = $true } }
        "frame" {
            $stop = @{ requestStop = $true } | ConvertTo-Json -Compress
            [Console]::Out.WriteLine($stop); [Console]::Out.Flush()
            $reply = @{ ok = $true }
        }
        "stop" { $reply = @{ ok = $true } }
        default { $reply = @{ ok = $false; error = "unknown phase" } }
    }
    [Console]::Out.WriteLine(($reply | ConvertTo-Json -Compress)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") { break }
}
"#,
    )
    .unwrap();

    let project_dir = temp_dir("project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"echo","version":"*"}],"entry":"src/main.txt"}"#).unwrap();
    let src_dir = project_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("main.txt"), "hello").unwrap();

    let output_dir = temp_dir("output");

    let bootstrap_exe = bootstrap_source::build_bootstrap(&|msg| println!("{msg}")).expect(
        "build_bootstrap failed — this test needs a `lowarc` checkout next to this repo; skip it if that's not the case here",
    );

    let options = ExportOptions {
        project_dir,
        modules_dir,
        output_dir,
        name: "Echo App".into(),
        diagnostics_log: false,
        target_fps: 30,
    };
    let exported = export::export_folder(&options, &bootstrap_exe, &|msg| println!("{msg}")).expect("export should succeed");

    let exe = exported.join("Echo App.exe");
    assert!(exe.is_file(), "expected the exported runtime at {}", exe.display());

    let status = std::process::Command::new(&exe).current_dir(&exported).status().expect("failed to launch the exported app");
    assert!(status.success(), "the exported app should run and exit cleanly, got {status:?}");

    let marker = exported.join("ran.txt");
    assert!(marker.is_file(), "expected the module to have actually run and written its marker file");
}
