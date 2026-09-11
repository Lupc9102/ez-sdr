//! Locates the vendor SDR libraries for whichever hardware feature is enabled, via
//! `pkg-config` when available. Falls back to a bare `-l<name>` link (already emitted by
//! the `#[link(name = "...")]` attribute on each backend's `extern "C"` block) when no
//! `.pc` file is found — e.g. a hand-installed library on a system's default linker search
//! path. We only warn (not fail the build) on a missing `.pc` file: `cargo check` must keep
//! working without the vendor headers installed, and a real `cargo build`/link will still
//! fail with a clear "unable to find library" error if the library truly isn't present.

fn probe(feature_env: &str, pkg_config_name: &str) {
    if std::env::var_os(feature_env).is_none() {
        return;
    }
    if let Err(err) = pkg_config::Config::new().probe(pkg_config_name) {
        println!(
            "cargo:warning=pkg-config could not find {pkg_config_name} ({err}); falling back \
             to the default linker search path"
        );
    }
}

fn main() {
    probe("CARGO_FEATURE_RTLSDR", "librtlsdr");
    probe("CARGO_FEATURE_HACKRF", "libhackrf");
    probe("CARGO_FEATURE_SOAPY", "SoapySDR");
}
