use crate::DeploymentError;
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub(crate) fn bytes(&mut self, len: usize) -> Result<&'a [u8], DeploymentError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(DeploymentError::MalformedPackage)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(DeploymentError::MalformedPackage)?;
        self.offset = end;
        Ok(value)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, DeploymentError> {
        Ok(*self
            .bytes(1)?
            .first()
            .ok_or(DeploymentError::MalformedPackage)?)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, DeploymentError> {
        let bytes: [u8; 2] = self
            .bytes(2)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)?;
        Ok(u16::from_le_bytes(bytes))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, DeploymentError> {
        let bytes: [u8; 4] = self
            .bytes(4)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub(crate) fn u32_as_usize(&mut self) -> Result<usize, DeploymentError> {
        usize::try_from(self.u32()?).map_err(|_| DeploymentError::MalformedPackage)
    }

    pub(crate) fn u64(&mut self) -> Result<u64, DeploymentError> {
        let bytes: [u8; 8] = self
            .bytes(8)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)?;
        Ok(u64::from_le_bytes(bytes))
    }

    pub(crate) fn array_32(&mut self) -> Result<[u8; 32], DeploymentError> {
        self.bytes(32)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)
    }

    pub(crate) fn array_64(&mut self) -> Result<[u8; 64], DeploymentError> {
        self.bytes(64)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)
    }

    pub(crate) fn finish(self) -> Result<(), DeploymentError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(DeploymentError::MalformedPackage)
        }
    }
}
