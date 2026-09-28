use kernel_auth::AuthorityDigest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableExternalFreshnessBinding {
    pub store_id: [u8; 32],
    pub previous_generation_digest: Option<AuthorityDigest>,
    pub trust_root_epoch: u64,
    pub deployment_policy_epoch: u64,
}
