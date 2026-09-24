use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use kernel_durability::{
    DestructiveDurabilityCut, SupportedDurabilityProfile, prepare_destructive_durability_case,
    verify_destructive_durability_case, verify_supported_durability_platform,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("cfmd durability campaign failed: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 3 {
        return Err(
            "usage: cfmd-durability-campaign <probe|arm|verify> <ext4|xfs> <directory> [cut]"
                .to_owned(),
        );
    }
    let command = &args[0];
    let profile = parse_profile(&args[1])?;
    let directory = PathBuf::from(&args[2]);
    let evidence = verify_supported_durability_platform(&directory, profile)
        .map_err(|error| format!("platform probe: {error:?}"))?;
    eprintln!(
        "PROFILE_OK profile={profile:?} kernel={} fs={} source={} options={:?}/{:?}",
        evidence.kernel_release,
        evidence.filesystem,
        evidence.mount_source,
        evidence.mount_options,
        evidence.super_options
    );
    match command.as_str() {
        "probe" => Ok(()),
        "arm" => {
            let cut = parse_cut(args.get(3).ok_or("arm requires cut")?)?;
            prepare_destructive_durability_case(&directory, cut)
                .map_err(|error| format!("prepare destructive case: {error:?}"))?;
            eprintln!(
                "ARMED cut={} -- perform HARD POWER CUT now; do not terminate normally",
                cut.as_str()
            );
            loop {
                thread::sleep(Duration::from_secs(3600));
            }
        }
        "verify" => {
            let cut = parse_cut(args.get(3).ok_or("verify requires cut")?)?;
            verify_destructive_durability_case(&directory, cut)
                .map_err(|error| format!("verify destructive case: {error:?}"))?;
            eprintln!("DESTRUCTIVE_CASE_PASS cut={}", cut.as_str());
            Ok(())
        }
        _ => Err("unknown command".to_owned()),
    }
}

fn parse_profile(value: &str) -> Result<SupportedDurabilityProfile, String> {
    match value {
        "ext4" => Ok(SupportedDurabilityProfile::LinuxExt4Ordered),
        "xfs" => Ok(SupportedDurabilityProfile::LinuxXfs),
        _ => Err("profile must be ext4 or xfs".to_owned()),
    }
}

fn parse_cut(value: &str) -> Result<DestructiveDurabilityCut, String> {
    DestructiveDurabilityCut::ALL
        .into_iter()
        .find(|cut| cut.as_str() == value)
        .ok_or_else(|| "unknown destructive cut".to_owned())
}
