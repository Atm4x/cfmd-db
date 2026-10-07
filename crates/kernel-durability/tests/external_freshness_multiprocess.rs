use std::env;
use std::fs;
use std::net::SocketAddr;
use std::process::{Child, Command};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use kernel_auth::{
    AuthorityDigest, FreshnessAuthorityState, FreshnessCut, TrustRootSet,
    freshness_authority_record_digest, verify_freshness_authority,
};
use kernel_durability::{
    ExternalFreshnessAuthority, TcpExternalFreshnessAuthority, TcpExternalFreshnessAuthorityServer,
};

const SERVER_ENV: &str = "CFMD_FRESHNESS_MP_SERVER";
const STATE_ENV: &str = "CFMD_FRESHNESS_MP_STATE";
const READY_ENV: &str = "CFMD_FRESHNESS_MP_READY";
const COUNT_ENV: &str = "CFMD_FRESHNESS_MP_COUNT";

#[test]
fn freshness_authority_worker() {
    if env::var_os(SERVER_ENV).is_none() {
        return;
    }
    let state = env::var(STATE_ENV).expect("state directory");
    let ready = env::var(READY_ENV).expect("ready file");
    let count = env::var(COUNT_ENV)
        .expect("request count")
        .parse::<usize>()
        .expect("numeric request count");
    let signing = SigningKey::from_bytes(&[73; 32]);
    let server = TcpExternalFreshnessAuthorityServer::bind(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        state,
        signing,
    )
    .expect("bind freshness authority");
    fs::write(
        ready,
        server.local_addr().expect("server address").to_string(),
    )
    .expect("publish server address");
    for _ in 0..count {
        server.serve_one().expect("serve freshness request");
    }
}

fn start_server(root: &std::path::Path, request_count: usize) -> (Child, SocketAddr) {
    let ready = root.join("ready");
    let _ = fs::remove_file(&ready);
    let child = Command::new(env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("freshness_authority_worker")
        .arg("--nocapture")
        .env(SERVER_ENV, "1")
        .env(STATE_ENV, root.join("authority-state"))
        .env(READY_ENV, &ready)
        .env(COUNT_ENV, request_count.to_string())
        .spawn()
        .expect("spawn freshness authority");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.is_file() {
        assert!(
            Instant::now() < deadline,
            "freshness authority did not start"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let addr = SocketAddr::from_str(fs::read_to_string(&ready).unwrap().trim()).unwrap();
    (child, addr)
}

fn cut(generation: u64, previous: Option<AuthorityDigest>, wal_lsn: u64) -> FreshnessCut {
    FreshnessCut {
        store_id: [0x31; 32],
        generation,
        previous_generation: previous,
        generation_digest: AuthorityDigest([u8::try_from(generation).unwrap(); 32]),
        wal_lsn,
        wal_digest: AuthorityDigest([u8::try_from(wal_lsn).unwrap_or(0xFE); 32]),
        trust_root_epoch: 9,
        deployment_policy_epoch: 4,
    }
}

#[test]
fn external_process_anchor_survives_restart_and_fences_regression() {
    let root = env::temp_dir().join(format!(
        "cfmd-freshness-mp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let signing = SigningKey::from_bytes(&[73; 32]);
    let trust = TrustRootSet::bootstrap(9, &[signing.verifying_key().to_bytes()]).unwrap();

    let (mut server, addr) = start_server(&root, 3);
    let mut authority =
        TcpExternalFreshnessAuthority::new(addr, Duration::from_secs(2), Duration::from_secs(2));
    let first = authority
        .compare_and_set_signed(None, FreshnessAuthorityState::DurableCut(cut(1, None, 0)))
        .unwrap();
    verify_freshness_authority(&trust, &first).unwrap();
    assert_eq!(authority.read_signed([0x31; 32]).unwrap().unwrap(), first);
    assert!(
        authority
            .compare_and_set_signed(
                Some(freshness_authority_record_digest(&first)),
                FreshnessAuthorityState::DurableCut(cut(0, None, 0))
            )
            .is_err()
    );
    assert!(server.wait().unwrap().success());
    assert!(authority.read_signed([0x31; 32]).is_err());

    let (mut restarted, addr) = start_server(&root, 2);
    let mut authority =
        TcpExternalFreshnessAuthority::new(addr, Duration::from_secs(2), Duration::from_secs(2));
    let recovered = authority.read_signed([0x31; 32]).unwrap().unwrap();
    assert_eq!(recovered, first);
    let recovered_cut = recovered.durable_cut().unwrap();
    let second = authority
        .compare_and_set_signed(
            Some(freshness_authority_record_digest(&recovered)),
            FreshnessAuthorityState::DurableCut(cut(2, Some(recovered_cut.generation_digest), 0)),
        )
        .unwrap();
    verify_freshness_authority(&trust, &second).unwrap();
    assert!(restarted.wait().unwrap().success());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_process_atomic_rebind_moves_trust_authority_across_store_identity() {
    let root = env::temp_dir().join(format!(
        "cfmd-freshness-rebind-mp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let signing = SigningKey::from_bytes(&[73; 32]);
    let trust = TrustRootSet::bootstrap(9, &[signing.verifying_key().to_bytes()]).unwrap();

    let source_id = [0x31; 32];
    let target_id = [0x42; 32];
    let (mut server, addr) = start_server(&root, 4);
    let mut authority =
        TcpExternalFreshnessAuthority::new(addr, Duration::from_secs(2), Duration::from_secs(2));
    let source = authority
        .compare_and_set_signed(None, FreshnessAuthorityState::DurableCut(cut(1, None, 0)))
        .unwrap();
    let source_cut = source.durable_cut().unwrap();
    let target_cut = FreshnessCut {
        store_id: target_id,
        generation: 1,
        previous_generation: Some(source_cut.generation_digest),
        generation_digest: AuthorityDigest([0x55; 32]),
        wal_lsn: 0,
        wal_digest: AuthorityDigest([0x66; 32]),
        trust_root_epoch: 9,
        deployment_policy_epoch: 4,
    };
    let target = authority
        .compare_and_rebind_signed(
            source_id,
            freshness_authority_record_digest(&source),
            FreshnessAuthorityState::DurableCut(target_cut),
        )
        .unwrap();
    verify_freshness_authority(&trust, &target).unwrap();
    assert!(authority.read_signed(source_id).unwrap().is_none());
    assert_eq!(authority.read_signed(target_id).unwrap().unwrap(), target);
    assert!(server.wait().unwrap().success());

    let (mut restarted, addr) = start_server(&root, 2);
    let mut authority =
        TcpExternalFreshnessAuthority::new(addr, Duration::from_secs(2), Duration::from_secs(2));
    assert!(authority.read_signed(source_id).unwrap().is_none());
    assert_eq!(authority.read_signed(target_id).unwrap().unwrap(), target);
    assert!(restarted.wait().unwrap().success());

    fs::remove_dir_all(root).unwrap();
}
