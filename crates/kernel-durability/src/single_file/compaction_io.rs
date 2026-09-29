use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SingleFileCompactionPrimitive {
    OpenRead,
    OpenReadWrite,
    MetadataLen,
    Seek,
    Read,
    Write,
    SyncData,
    SyncAll,
    SetLen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SingleFileCompactionPublicationBoundary {
    SourceRootWriteUncertain,
    SourceRootSynced,
    StagingImageSynced,
    StagingRootWriteUncertain,
    StagingRootSynced,
    FrontImageSynced,
    FinalRootWriteUncertain,
    FinalRootSynced,
    TailReclaimSynced,
    JournalReopenRootWriteUncertain,
    JournalReopenRootSynced,
}

impl SingleFileCompactionPublicationBoundary {
    #[cfg(test)]
    pub(crate) const fn is_sync_boundary(self) -> bool {
        !matches!(
            self,
            Self::SourceRootWriteUncertain
                | Self::StagingRootWriteUncertain
                | Self::FinalRootWriteUncertain
                | Self::JournalReopenRootWriteUncertain
        )
    }

    #[cfg(test)]
    pub(crate) const fn is_uncertain_root_write(self) -> bool {
        matches!(
            self,
            Self::SourceRootWriteUncertain
                | Self::StagingRootWriteUncertain
                | Self::FinalRootWriteUncertain
                | Self::JournalReopenRootWriteUncertain
        )
    }

    #[cfg(test)]
    pub(crate) const fn crash_name(self) -> &'static str {
        match self {
            Self::SourceRootWriteUncertain => "after-single-file-sealed-root-write-before-sync",
            Self::SourceRootSynced => "after-single-file-sealed-root-sync",
            Self::StagingImageSynced => "after-single-file-staging-image-sync",
            Self::StagingRootWriteUncertain => "after-single-file-staging-root-write-before-sync",
            Self::StagingRootSynced => "after-single-file-staging-root-sync",
            Self::FrontImageSynced => "after-single-file-front-image-sync",
            Self::FinalRootWriteUncertain => "after-single-file-final-root-write-before-sync",
            Self::FinalRootSynced => "after-single-file-final-root-sync",
            Self::TailReclaimSynced => "after-single-file-tail-reclaim-sync",
            Self::JournalReopenRootWriteUncertain => {
                "after-single-file-journal-reopen-root-write-before-sync"
            }
            Self::JournalReopenRootSynced => "after-single-file-journal-reopen-root-sync",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SingleFileCompactionIoLaw {
    primitive: SingleFileCompactionPrimitive,
    publication: Option<SingleFileCompactionPublicationBoundary>,
}

macro_rules! publication_boundary {
    () => {
        None
    };
    ($boundary:ident) => {
        Some(SingleFileCompactionPublicationBoundary::$boundary)
    };
}

macro_rules! define_compaction_io_steps {
    ($( $step:ident => $primitive:ident $(, $boundary:ident)?; )+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum SingleFileCompactionIoStep {
            $( $step, )+
        }

        impl SingleFileCompactionIoStep {
            #[cfg(test)]
            pub(crate) const ALL: &[Self] = &[
                $( Self::$step, )+
            ];

            pub(crate) const fn law(self) -> SingleFileCompactionIoLaw {
                match self {
                    $(
                        Self::$step => SingleFileCompactionIoLaw {
                            primitive: SingleFileCompactionPrimitive::$primitive,
                            publication: publication_boundary!($($boundary)?),
                        },
                    )+
                }
            }
        }
    };
}

define_compaction_io_steps! {
    SourceWalSync => SyncData;
    SourceContainerSync => SyncAll;
    SourceLength => MetadataLen;
    SealedRootWrite => Write, SourceRootWriteUncertain;
    SealedRootSync => SyncAll, SourceRootSynced;
    SourceValidationSeek => Seek;
    SourceValidationRead => Read;
    StagingLength => MetadataLen;
    StagingSourceOpen => OpenRead;
    StagingDestinationSeek => Seek;
    StagingSourceSeek => Seek;
    StagingImageRead => Read;
    StagingImageWrite => Write;
    StagingImageSync => SyncAll, StagingImageSynced;
    StagingValidationSeek => Seek;
    StagingValidationRead => Read;
    StagingValidationOpen => OpenReadWrite;
    StagingWalSeek => Seek;
    StagingWalRead => Read;
    StagingRootWrite => Write, StagingRootWriteUncertain;
    StagingRootSync => SyncAll, StagingRootSynced;
    FrontSourceOpen => OpenRead;
    FrontDestinationSeek => Seek;
    FrontSourceSeek => Seek;
    FrontImageRead => Read;
    FrontImageWrite => Write;
    FrontImageSync => SyncAll, FrontImageSynced;
    FrontValidationSeek => Seek;
    FrontValidationRead => Read;
    FrontValidationOpen => OpenReadWrite;
    FrontWalSeek => Seek;
    FrontWalRead => Read;
    FinalRootWrite => Write, FinalRootWriteUncertain;
    FinalRootSync => SyncAll, FinalRootSynced;
    TailLength => MetadataLen;
    TailTruncate => SetLen;
    TailSync => SyncAll, TailReclaimSynced;
    JournalReopenOpen => OpenReadWrite;
    JournalReopenWalSeek => Seek;
    JournalReopenWalRead => Read;
    JournalReopenRootWrite => Write, JournalReopenRootWriteUncertain;
    JournalReopenRootSync => SyncAll, JournalReopenRootSynced;
}

impl SingleFileCompactionIoStep {
    pub(crate) const fn primitive(self) -> SingleFileCompactionPrimitive {
        self.law().primitive
    }

    #[cfg(test)]
    pub(crate) const fn publication_boundary(
        self,
    ) -> Option<SingleFileCompactionPublicationBoundary> {
        self.law().publication
    }

    #[cfg(test)]
    pub(crate) const fn supports_short_progress(self) -> bool {
        matches!(
            self.primitive(),
            SingleFileCompactionPrimitive::Read | SingleFileCompactionPrimitive::Write
        )
    }

    #[cfg(test)]
    pub(crate) const fn supports_zero_progress(self) -> bool {
        matches!(self.primitive(), SingleFileCompactionPrimitive::Write)
    }

    #[cfg(test)]
    pub(crate) const fn is_publication_sync_boundary(self) -> bool {
        match self.publication_boundary() {
            Some(boundary) => boundary.is_sync_boundary(),
            None => false,
        }
    }

    #[cfg(test)]
    pub(crate) const fn is_uncertain_root_write(self) -> bool {
        match self.publication_boundary() {
            Some(boundary) => boundary.is_uncertain_root_write(),
            None => false,
        }
    }

    #[cfg(test)]
    pub(crate) const fn crash_name(self) -> Option<&'static str> {
        match self.publication_boundary() {
            Some(boundary) => Some(boundary.crash_name()),
            None => None,
        }
    }
}

fn assert_primitive(step: SingleFileCompactionIoStep, expected: SingleFileCompactionPrimitive) {
    debug_assert_eq!(step.primitive(), expected);
}

pub(crate) trait SingleFileCompactionIo {
    fn open_read(&mut self, step: SingleFileCompactionIoStep, path: &Path) -> io::Result<File> {
        assert_primitive(step, SingleFileCompactionPrimitive::OpenRead);
        File::open(path)
    }

    fn open_read_write(
        &mut self,
        step: SingleFileCompactionIoStep,
        path: &Path,
    ) -> io::Result<File> {
        assert_primitive(step, SingleFileCompactionPrimitive::OpenReadWrite);
        OpenOptions::new().read(true).write(true).open(path)
    }

    fn metadata_len(&mut self, step: SingleFileCompactionIoStep, file: &File) -> io::Result<u64> {
        assert_primitive(step, SingleFileCompactionPrimitive::MetadataLen);
        Ok(file.metadata()?.len())
    }

    fn seek(
        &mut self,
        step: SingleFileCompactionIoStep,
        file: &mut File,
        position: SeekFrom,
    ) -> io::Result<u64> {
        assert_primitive(step, SingleFileCompactionPrimitive::Seek);
        file.seek(position)
    }

    fn read(
        &mut self,
        step: SingleFileCompactionIoStep,
        file: &mut File,
        bytes: &mut [u8],
    ) -> io::Result<usize> {
        assert_primitive(step, SingleFileCompactionPrimitive::Read);
        file.read(bytes)
    }

    fn write(
        &mut self,
        step: SingleFileCompactionIoStep,
        file: &mut File,
        bytes: &[u8],
    ) -> io::Result<usize> {
        assert_primitive(step, SingleFileCompactionPrimitive::Write);
        file.write(bytes)
    }

    fn sync_data(&mut self, step: SingleFileCompactionIoStep, file: &File) -> io::Result<()> {
        assert_primitive(step, SingleFileCompactionPrimitive::SyncData);
        file.sync_data()
    }

    fn sync_all(&mut self, step: SingleFileCompactionIoStep, file: &File) -> io::Result<()> {
        assert_primitive(step, SingleFileCompactionPrimitive::SyncAll);
        file.sync_all()
    }

    fn set_len(
        &mut self,
        step: SingleFileCompactionIoStep,
        file: &File,
        len: u64,
    ) -> io::Result<()> {
        assert_primitive(step, SingleFileCompactionPrimitive::SetLen);
        file.set_len(len)
    }
}

pub(crate) struct OsSingleFileCompactionIo;
impl SingleFileCompactionIo for OsSingleFileCompactionIo {}

pub(crate) struct SingleFileCompactionReader<'a, I> {
    pub(crate) io: &'a mut I,
    pub(crate) file: &'a mut File,
    pub(crate) read_step: SingleFileCompactionIoStep,
    pub(crate) seek_step: SingleFileCompactionIoStep,
}

impl<I: SingleFileCompactionIo> Read for SingleFileCompactionReader<'_, I> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.io.read(self.read_step, self.file, bytes)
    }
}

impl<I: SingleFileCompactionIo> Seek for SingleFileCompactionReader<'_, I> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.io.seek(self.seek_step, self.file, position)
    }
}

pub(crate) fn write_all(
    io: &mut impl SingleFileCompactionIo,
    step: SingleFileCompactionIoStep,
    file: &mut File,
    mut bytes: &[u8],
) -> io::Result<()> {
    while !bytes.is_empty() {
        match io.write(step, file, bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "compaction write made no progress",
                ));
            }
            Ok(written) if written <= bytes.len() => bytes = &bytes[written..],
            Ok(_) => {
                return Err(io::Error::other(
                    "compaction write reported an impossible length",
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct CopyRangeSteps {
    pub(crate) seek: SingleFileCompactionIoStep,
    pub(crate) read: SingleFileCompactionIoStep,
    pub(crate) write: SingleFileCompactionIoStep,
}

pub(crate) fn copy_exact_range(
    io: &mut impl SingleFileCompactionIo,
    steps: CopyRangeSteps,
    source: &mut File,
    offset: u64,
    len: u64,
    destination: &mut File,
) -> io::Result<()> {
    io.seek(steps.seek, source, SeekFrom::Start(offset))?;
    let mut remaining = len;
    let mut buffer = [0_u8; 16 * 1024];
    while remaining != 0 {
        let chunk = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| io::Error::other("compaction range length exceeds usize"))?;
        let mut filled = 0;
        while filled < chunk {
            match io.read(steps.read, source, &mut buffer[filled..chunk]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "compaction source range is truncated",
                    ));
                }
                Ok(read) if read <= chunk - filled => filled += read,
                Ok(_) => {
                    return Err(io::Error::other(
                        "compaction read reported an impossible length",
                    ));
                }
            }
        }
        write_all(io, steps.write, destination, &buffer[..chunk])?;
        remaining -= chunk as u64;
    }
    Ok(())
}
