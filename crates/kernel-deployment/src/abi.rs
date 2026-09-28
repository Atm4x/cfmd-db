use crate::cursor::Cursor;
use crate::limits::ABI_VERSION;
use crate::{DeploymentError, RuntimeProfileSpec};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiOperation {
    Equivalence,
    Ordering,
    Tokenize,
}

impl AbiOperation {
    const fn tag(self) -> u8 {
        match self {
            Self::Equivalence => 1,
            Self::Ordering => 2,
            Self::Tokenize => 3,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, DeploymentError> {
        match tag {
            1 => Ok(Self::Equivalence),
            2 => Ok(Self::Ordering),
            3 => Ok(Self::Tokenize),
            _ => Err(DeploymentError::MalformedAbiFrame),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiRequest {
    pub operation: AbiOperation,
    pub fuel_limit: u64,
    pub payload: Vec<u8>,
}

impl AbiRequest {
    pub fn encode(&self, profile: RuntimeProfileSpec) -> Result<Vec<u8>, DeploymentError> {
        profile.validate()?;
        if self.fuel_limit == 0 || self.fuel_limit > profile.max_fuel {
            return Err(DeploymentError::FuelLimitExceeded);
        }
        let payload_len =
            u32::try_from(self.payload.len()).map_err(|_| DeploymentError::AbiFrameTooLarge)?;
        let mut out = Vec::with_capacity(15 + self.payload.len());
        out.extend_from_slice(&ABI_VERSION.to_le_bytes());
        out.push(self.operation.tag());
        out.extend_from_slice(&self.fuel_limit.to_le_bytes());
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&self.payload);
        if out.len() > usize::try_from(profile.max_request_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8], profile: RuntimeProfileSpec) -> Result<Self, DeploymentError> {
        profile.validate()?;
        if bytes.len() > usize::try_from(profile.max_request_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        let mut cursor = Cursor::new(bytes);
        let version = cursor.u16()?;
        if version != ABI_VERSION {
            return Err(DeploymentError::AbiVersionMismatch);
        }
        let operation = AbiOperation::from_tag(cursor.u8()?)?;
        let fuel_limit = cursor.u64()?;
        if fuel_limit == 0 || fuel_limit > profile.max_fuel {
            return Err(DeploymentError::FuelLimitExceeded);
        }
        let payload_len = cursor.u32_as_usize()?;
        let payload = cursor.bytes(payload_len)?.to_vec();
        cursor.finish()?;
        Ok(Self {
            operation,
            fuel_limit,
            payload,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiResponse {
    pub consumed_fuel: u64,
    pub payload: Vec<u8>,
}

impl AbiResponse {
    pub fn encode(&self, profile: RuntimeProfileSpec) -> Result<Vec<u8>, DeploymentError> {
        profile.validate()?;
        if self.consumed_fuel > profile.max_fuel {
            return Err(DeploymentError::FuelLimitExceeded);
        }
        let payload_len =
            u32::try_from(self.payload.len()).map_err(|_| DeploymentError::AbiFrameTooLarge)?;
        let mut out = Vec::with_capacity(14 + self.payload.len());
        out.extend_from_slice(&ABI_VERSION.to_le_bytes());
        out.extend_from_slice(&self.consumed_fuel.to_le_bytes());
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&self.payload);
        if out.len() > usize::try_from(profile.max_response_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8], profile: RuntimeProfileSpec) -> Result<Self, DeploymentError> {
        profile.validate()?;
        if bytes.len() > usize::try_from(profile.max_response_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        let mut cursor = Cursor::new(bytes);
        if cursor.u16()? != ABI_VERSION {
            return Err(DeploymentError::AbiVersionMismatch);
        }
        let consumed_fuel = cursor.u64()?;
        if consumed_fuel > profile.max_fuel {
            return Err(DeploymentError::FuelLimitExceeded);
        }
        let payload_len = cursor.u32_as_usize()?;
        let payload = cursor.bytes(payload_len)?.to_vec();
        cursor.finish()?;
        Ok(Self {
            consumed_fuel,
            payload,
        })
    }
}

pub trait SandboxedRuntime {
    fn profile(&self) -> RuntimeProfileSpec;

    fn invoke(
        &mut self,
        artifact: &[u8],
        encoded_request: &[u8],
    ) -> Result<Vec<u8>, DeploymentError>;
}
