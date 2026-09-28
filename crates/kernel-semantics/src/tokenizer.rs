use kernel_schema::ModuleDigest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerModule {
    AsciiWhitespace,
    AsciiWhitespaceLowercase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TokenizerImplementation {
    pub(super) contract: TokenizerModule,
    pub(super) implementation_revision: u64,
}

impl TokenizerModule {
    #[must_use]
    pub(super) fn implementation_digest(self, implementation_revision: u64) -> ModuleDigest {
        TokenizerImplementation {
            contract: self,
            implementation_revision,
        }
        .digest()
    }

    #[must_use]
    pub fn digest(self) -> ModuleDigest {
        match self {
            Self::AsciiWhitespace => ModuleDigest([21; 32]),
            Self::AsciiWhitespaceLowercase => ModuleDigest([22; 32]),
        }
    }

    #[must_use]
    pub fn tokenize(self, text: &str) -> Vec<String> {
        match self {
            Self::AsciiWhitespace => text.split_ascii_whitespace().map(str::to_owned).collect(),
            Self::AsciiWhitespaceLowercase => text
                .split_ascii_whitespace()
                .map(str::to_ascii_lowercase)
                .collect(),
        }
    }
}

impl TokenizerImplementation {
    #[must_use]
    pub(super) fn digest(self) -> ModuleDigest {
        let mut digest = self.contract.digest().0;
        let revision = self.implementation_revision.to_le_bytes();
        for (index, byte) in revision.into_iter().enumerate() {
            digest[24 + index] ^= byte;
        }
        ModuleDigest(digest)
    }
}
