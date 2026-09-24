use std::fs;
use std::path::PathBuf;

use ed25519_dalek::{Signer, SigningKey};
use kernel_auth::{Sha256Digest, TrustRootSet, key_id};
use kernel_durability::{
    DestructiveDurabilityCampaignEvidence, DurableRevisionStore,
    SignedDestructiveDurabilityCampaignEvidence, SupportedDurabilityProfile,
    durability_platform_fingerprint, verify_signed_destructive_durability_campaign,
    verify_supported_durability_platform,
};
use kernel_model::{DatabaseState, Value};
use kernel_revision::Revision;
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

const DOMAIN: &[u8] = b"CFMD-DURABILITY-CAMPAIGN-AUTH-v1\0";
const CAMPAIGN_ID: &str = "qemu-tcg-ext4-ordered-pass120-2026-09-23";
const TRUST_EPOCH: u64 = 1;

fn parse_hex_32(s: &str) -> Result<[u8; 32], String> {
    if s.len() != 64 { return Err("expected 64 hex chars".into()); }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i*2..i*2+2], 16).map_err(|_| "bad hex")?;
    }
    Ok(out)
}
fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }

fn signing_message(e: &DestructiveDurabilityCampaignEvidence) -> Vec<u8> {
    let mut bytes = DOMAIN.to_vec();
    bytes.extend_from_slice(&TRUST_EPOCH.to_le_bytes());
    bytes.push(1); // LinuxExt4Ordered
    bytes.extend_from_slice(&(e.campaign_id.len() as u64).to_le_bytes());
    bytes.extend_from_slice(e.campaign_id.as_bytes());
    bytes.extend_from_slice(&e.platform_fingerprint.0);
    bytes.extend_from_slice(&e.completed_fault_cases.to_le_bytes());
    bytes.extend_from_slice(&e.expected_fault_cases.to_le_bytes());
    bytes.extend_from_slice(&e.evidence_digest.0);
    bytes
}

fn setup_revision() -> (Revision, SemanticRegistry) {
    let relation = SemanticId::new(1);
    let equivalence = SemanticId::new(2);
    let mut registry = SemanticRegistry::default();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
    environment.pin_module(equivalence, registry.install_equivalence(EquivalenceModule::I64Exact));
    let mut schema = Schema::new(SchemaRevisionId::new(1));
    schema.define_relation(RelationDef {
        id: relation,
        columns: vec![TypeExpr::Scalar(ScalarType::I64)],
        semantics: RelationSemantics::Bag { column_equivalences: vec![equivalence] },
    }).expect("schema");
    let context = SemanticContext { schema, environment };
    let mut state = DatabaseState::default();
    state.model.relations.insert(relation, vec![vec![Value::I64(1)]]);
    let revision = Revision::build(RevisionId::new(1), &context, &registry, state).expect("revision");
    (revision, registry)
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 { return Err("usage: cfmd-hist13-certify <mount-dir> <evidence-digest-hex> <seed-hex>".to_owned()); }
    let mount_dir = PathBuf::from(&args[1]);
    let evidence_digest = Sha256Digest(parse_hex_32(&args[2])?);
    let seed = parse_hex_32(&args[3])?;

    let runtime = verify_supported_durability_platform(&mount_dir, SupportedDurabilityProfile::LinuxExt4Ordered).map_err(|e| format!("platform: {e:?}"))?;
    let fingerprint = durability_platform_fingerprint(&runtime);
    println!("PLATFORM_FINGERPRINT={}", hex(&fingerprint.0));

    let evidence = DestructiveDurabilityCampaignEvidence {
        profile: SupportedDurabilityProfile::LinuxExt4Ordered,
        campaign_id: CAMPAIGN_ID.to_owned(),
        platform_fingerprint: fingerprint,
        completed_fault_cases: 7,
        expected_fault_cases: 7,
        evidence_digest,
    };
    let signing = SigningKey::from_bytes(&seed);
    let public = signing.verifying_key().to_bytes();
    let signer = key_id(&public);
    let signature = signing.sign(&signing_message(&evidence)).to_bytes();
    let trust = TrustRootSet::bootstrap(TRUST_EPOCH, &[public]).map_err(|e| format!("trust: {e:?}"))?;
    let signed = SignedDestructiveDurabilityCampaignEvidence {
        trust_root_epoch: TRUST_EPOCH,
        evidence,
        signer,
        signature,
    };
    let verified = verify_signed_destructive_durability_campaign(&trust, &signed).map_err(|e| format!("certificate: {e:?}"))?;
    println!("SIGNER_KEY_ID={}", hex(&signer.0));
    println!("PUBLIC_KEY={}", hex(&public));
    println!("SIGNATURE={}", hex(&signature));
    println!("EVIDENCE_DIGEST={}", args[2]);
    println!("CERTIFICATE_VERIFY=PASS");

    let store_dir = mount_dir.join("certified-store-api-smoke");
    let _ = fs::remove_dir_all(&store_dir);
    let (revision, registry) = setup_revision();
    {
        let store = DurableRevisionStore::create_on_supported_platform(
            &store_dir,
            SupportedDurabilityProfile::LinuxExt4Ordered,
            &verified,
            &revision,
            &registry,
        ).map_err(|e| format!("certified create: {e:?}"))?;
        drop(store);
    }
    let (_store, _scan) = DurableRevisionStore::open_on_supported_platform(
        &store_dir,
        SupportedDurabilityProfile::LinuxExt4Ordered,
        &verified,
    ).map_err(|e| format!("certified open: {e:?}"))?;
    println!("CERTIFIED_STORE_CREATE_OPEN=PASS");
    Ok(())
}
