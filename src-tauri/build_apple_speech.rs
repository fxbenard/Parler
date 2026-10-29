// Apple SpeechAnalyzer bridge builder (macOS 26+ Speech framework).
//
// Compiles swift/apple_speech.swift into a static library with a C ABI.
// When the SDK predates SpeechAnalyzer, a stub exporting the same symbols is
// compiled instead so linking always succeeds.

fn build_apple_speech_bridge() {
    use std::env;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const REAL_SWIFT_FILE: &str = "swift/apple_speech.swift";
    const STUB_SWIFT_FILE: &str = "swift/apple_speech_stub.swift";
    const BRIDGE_HEADER: &str = "swift/apple_speech_bridge.h";

    println!("cargo:rerun-if-changed={REAL_SWIFT_FILE}");
    println!("cargo:rerun-if-changed={STUB_SWIFT_FILE}");
    println!("cargo:rerun-if-changed={BRIDGE_HEADER}");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let object_path = out_dir.join("apple_speech.o");
    let static_lib_path = out_dir.join("libapple_speech.a");

    let sdk_path = String::from_utf8(
        Command::new("xcrun")
            .args(["--sdk", "macosx", "--show-sdk-path"])
            .output()
            .expect("Failed to locate macOS SDK")
            .stdout,
    )
    .expect("SDK path is not valid UTF-8")
    .trim()
    .to_string();

    let swiftc_path = String::from_utf8(
        Command::new("xcrun")
            .args(["--find", "swiftc"])
            .output()
            .expect("Failed to locate swiftc")
            .stdout,
    )
    .expect("swiftc path is not valid UTF-8")
    .trim()
    .to_string();

    // SpeechAnalyzer lives in the Speech framework's Swift interface; detect it
    // to decide between the real implementation and the stub.
    let speech_interface = Path::new(&sdk_path).join(
        "System/Library/Frameworks/Speech.framework/Modules/Speech.swiftmodule/arm64e-apple-macos.swiftinterface",
    );
    let has_speech_analyzer = std::fs::read_to_string(&speech_interface)
        .map(|content| content.contains("SpeechAnalyzer"))
        .unwrap_or(false);

    let source_file = if has_speech_analyzer {
        println!("cargo:warning=Building with Apple SpeechAnalyzer support.");
        REAL_SWIFT_FILE
    } else {
        println!("cargo:warning=SpeechAnalyzer SDK not found. Building Apple Speech stub.");
        STUB_SWIFT_FILE
    };

    if !Path::new(source_file).exists() {
        panic!("Source file {} is missing!", source_file);
    }

    let toolchain_swift_lib = Path::new(&swiftc_path)
        .parent()
        .and_then(|p| p.parent())
        .map(|root| root.join("lib/swift/macosx"))
        .expect("Unable to determine Swift toolchain lib directory");
    let sdk_swift_lib = Path::new(&sdk_path).join("usr/lib/swift");

    // macOS 11 deployment target: @available(macOS 26.0, *) guards in Swift and
    // weak framework linking keep older systems working.
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-target",
            "arm64-apple-macosx11.0",
            "-sdk",
            &sdk_path,
            "-O",
            "-import-objc-header",
            BRIDGE_HEADER,
            "-c",
            source_file,
            "-o",
            object_path
                .to_str()
                .expect("Failed to convert object path to string"),
        ])
        .status()
        .expect("Failed to invoke swiftc for Apple Speech bridge");

    if !status.success() {
        panic!("swiftc failed to compile {source_file}");
    }

    let status = Command::new("libtool")
        .args([
            "-static",
            "-o",
            static_lib_path
                .to_str()
                .expect("Failed to convert static lib path to string"),
            object_path
                .to_str()
                .expect("Failed to convert object path to string"),
        ])
        .status()
        .expect("Failed to create static library for Apple Speech bridge");

    if !status.success() {
        panic!("libtool failed for Apple Speech bridge");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=apple_speech");
    println!(
        "cargo:rustc-link-search=native={}",
        toolchain_swift_lib.display()
    );
    println!("cargo:rustc-link-search=native={}", sdk_swift_lib.display());
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=AVFoundation");

    // Weak link Speech: the app must launch on macOS < 26 where the
    // SpeechAnalyzer symbols do not exist.
    println!("cargo:rustc-link-arg=-weak_framework");
    println!("cargo:rustc-link-arg=Speech");

    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
