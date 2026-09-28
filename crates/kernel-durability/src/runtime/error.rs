use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableFormatComponent {
    Manifest,
    CheckpointFile,
    MetadataFile,
}

#[derive(Debug)]
pub enum DurabilityError {
    Io(io::Error),
    Poisoned,
    LsnExhausted,
    PayloadTooLarge,
    UnsupportedDurableFormat {
        component: DurableFormatComponent,
        version: u16,
    },
    Encode(CodecError),
    Corruption {
        offset: usize,
        reason: &'static str,
    },
    Protocol {
        offset: usize,
        reason: &'static str,
    },
}

impl PartialEq for DurabilityError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            (Self::Poisoned, Self::Poisoned)
            | (Self::LsnExhausted, Self::LsnExhausted)
            | (Self::PayloadTooLarge, Self::PayloadTooLarge) => true,
            (
                Self::UnsupportedDurableFormat {
                    component: left_component,
                    version: left_version,
                },
                Self::UnsupportedDurableFormat {
                    component: right_component,
                    version: right_version,
                },
            ) => left_component == right_component && left_version == right_version,
            (Self::Encode(left), Self::Encode(right)) => left == right,
            (
                Self::Corruption {
                    offset: left_offset,
                    reason: left_reason,
                },
                Self::Corruption {
                    offset: right_offset,
                    reason: right_reason,
                },
            )
            | (
                Self::Protocol {
                    offset: left_offset,
                    reason: left_reason,
                },
                Self::Protocol {
                    offset: right_offset,
                    reason: right_reason,
                },
            ) => left_offset == right_offset && left_reason == right_reason,
            _ => false,
        }
    }
}

impl Eq for DurabilityError {}

impl From<io::Error> for DurabilityError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<CodecError> for DurabilityError {
    fn from(value: CodecError) -> Self {
        Self::Encode(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    LengthOverflow,
    CollectionTooLarge,
    ValueNestingTooDeep,
    SemanticModuleUnavailable,
}
