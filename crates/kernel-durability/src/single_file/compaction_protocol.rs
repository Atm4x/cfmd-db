use std::marker::PhantomData;

use crate::runtime::DurabilityError;

use super::compaction_io::SingleFileCompactionIoStep;

pub(super) struct SourceUnsealed;
pub(super) struct SourceRootUncertain;
pub(super) struct SourceSealed;
pub(super) struct StagingImageDurable;
pub(super) struct StagingRootUncertain;
pub(super) struct StagingRootDurable;
pub(super) struct FrontImageDurable;
pub(super) struct FinalRootUncertain;
pub(super) struct FinalRootDurable;
pub(super) struct TailReclaimed;
pub(super) struct JournalReopenRootUncertain;
pub(super) struct JournalReopened;

pub(super) trait PublicationTransition {
    type Next;
    const STEP: SingleFileCompactionIoStep;
}

macro_rules! define_publication_transitions {
    ($( $from:ident => $to:ident @ $step:ident; )+) => {
        $(
            impl PublicationTransition for $from {
                type Next = $to;
                const STEP: SingleFileCompactionIoStep = SingleFileCompactionIoStep::$step;
            }
        )+

        #[cfg(test)]
        pub(super) const PUBLICATION_SEQUENCE: &[SingleFileCompactionIoStep] = &[
            $( SingleFileCompactionIoStep::$step, )+
        ];
    };
}

define_publication_transitions! {
    SourceUnsealed => SourceRootUncertain @ SealedRootWrite;
    SourceRootUncertain => SourceSealed @ SealedRootSync;
    SourceSealed => StagingImageDurable @ StagingImageSync;
    StagingImageDurable => StagingRootUncertain @ StagingRootWrite;
    StagingRootUncertain => StagingRootDurable @ StagingRootSync;
    StagingRootDurable => FrontImageDurable @ FrontImageSync;
    FrontImageDurable => FinalRootUncertain @ FinalRootWrite;
    FinalRootUncertain => FinalRootDurable @ FinalRootSync;
    FinalRootDurable => TailReclaimed @ TailSync;
    TailReclaimed => JournalReopenRootUncertain @ JournalReopenRootWrite;
    JournalReopenRootUncertain => JournalReopened @ JournalReopenRootSync;
}

pub(super) struct Publication<'a, F, S> {
    fault: &'a mut F,
    state: PhantomData<S>,
}

impl<'a, F> Publication<'a, F, SourceUnsealed> {
    pub(super) fn new(fault: &'a mut F) -> Self {
        Self {
            fault,
            state: PhantomData,
        }
    }
}

impl<'a, F, S> Publication<'a, F, S>
where
    F: FnMut(SingleFileCompactionIoStep) -> Result<(), DurabilityError>,
    S: PublicationTransition,
{
    pub(super) fn advance_after(
        self,
        operation: impl FnOnce(SingleFileCompactionIoStep) -> Result<(), DurabilityError>,
    ) -> Result<Publication<'a, F, S::Next>, DurabilityError> {
        operation(S::STEP)?;
        (self.fault)(S::STEP)?;
        Ok(Publication {
            fault: self.fault,
            state: PhantomData,
        })
    }
}
