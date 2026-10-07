//! Unix-domain-socket implementation of the local IPC transport.

use std::{
    fs,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
};

use cfmd_host::{Authenticator, Authorizer, HostedConnection, HostedServer};
use cfmd_protocol::{
    ProtocolErrorCode,
    wire::{WIRE_HEADER_LEN, WireLimits, decode_header, encode_error_response_frame},
};

use crate::{
    LOCAL_AUTH_HEADER_LEN, LOCAL_AUTH_MAGIC, LOCAL_AUTH_VERSION, LocalIpcLimits, Result,
    auth_evidence_too_large, invalid_auth_prelude, resource_limit,
};

pub struct UnixLocalIpcServer<A, Z>
where
    A: Authenticator<Vec<u8>>,
    Z: Authorizer,
{
    listener: UnixListener,
    endpoint: PathBuf,
    host: HostedServer<Vec<u8>, A, Z>,
    limits: LocalIpcLimits,
}

impl<A, Z> UnixLocalIpcServer<A, Z>
where
    A: Authenticator<Vec<u8>>,
    Z: Authorizer,
{
    pub fn bind(
        endpoint: impl AsRef<Path>,
        host: HostedServer<Vec<u8>, A, Z>,
        limits: LocalIpcLimits,
    ) -> Result<Self> {
        validate_limits(limits, host.limits().max_in_flight_requests_per_connection)?;
        let endpoint = endpoint.as_ref().to_path_buf();
        let listener = UnixListener::bind(&endpoint)?;
        if let Err(error) = fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)) {
            drop(listener);
            let _ = fs::remove_file(&endpoint);
            return Err(error.into());
        }
        Ok(Self {
            listener,
            endpoint,
            host,
            limits,
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }

    #[must_use]
    pub const fn limits(&self) -> LocalIpcLimits {
        self.limits
    }

    pub fn accept_one(&self) -> Result<()> {
        let (stream, _) = self.listener.accept()?;
        self.serve_stream(stream)
    }

    pub fn serve_stream(&self, mut stream: UnixStream) -> Result<()> {
        stream.set_read_timeout(Some(self.limits.authentication_read_timeout))?;
        let evidence = read_authentication_evidence(&mut stream, self.limits)?;
        stream.set_read_timeout(None)?;
        let connection = self.host.connect(&evidence)?;
        serve_authenticated_stream(
            stream,
            &connection,
            self.host.limits().wire,
            self.limits,
            self.host.limits().max_in_flight_requests_per_connection,
        )
    }
}

pub struct UnixLocalIpcClient {
    stream: UnixStream,
    wire_limits: WireLimits,
}

impl UnixLocalIpcClient {
    pub fn connect(
        endpoint: impl AsRef<Path>,
        evidence: &[u8],
        local_limits: LocalIpcLimits,
        wire_limits: WireLimits,
    ) -> Result<Self> {
        if evidence.len()
            > usize::try_from(local_limits.max_auth_evidence_bytes).unwrap_or(usize::MAX)
        {
            return Err(auth_evidence_too_large());
        }
        let evidence_len = u32::try_from(evidence.len()).map_err(|_| auth_evidence_too_large())?;
        let mut stream = UnixStream::connect(endpoint)?;
        let mut header = [0_u8; LOCAL_AUTH_HEADER_LEN];
        header[..4].copy_from_slice(&LOCAL_AUTH_MAGIC);
        header[4..6].copy_from_slice(&LOCAL_AUTH_VERSION.to_be_bytes());
        header[8..12].copy_from_slice(&evidence_len.to_be_bytes());
        stream.write_all(&header)?;
        stream.write_all(evidence)?;
        Ok(Self {
            stream,
            wire_limits,
        })
    }

    pub fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        self.stream.write_all(frame)?;
        Ok(())
    }

    pub fn receive_frame(&mut self) -> Result<Vec<u8>> {
        read_wire_frame(&mut self.stream, self.wire_limits)?.ok_or_else(|| {
            invalid_auth_prelude("local IPC peer closed before returning a wire response")
        })
    }

    pub fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        self.send_frame(frame)?;
        self.receive_frame()
    }

    pub fn shutdown(&self) -> Result<()> {
        self.stream.shutdown(Shutdown::Both)?;
        Ok(())
    }
}

fn validate_limits(limits: LocalIpcLimits, host_in_flight_limit: usize) -> Result<()> {
    if limits.max_auth_evidence_bytes == 0
        || limits.max_workers_per_connection == 0
        || limits.max_pending_responses_per_connection == 0
        || limits.authentication_read_timeout.is_zero()
        || host_in_flight_limit == 0
    {
        return Err(resource_limit("local IPC limits must be non-zero"));
    }
    Ok(())
}

fn read_authentication_evidence(
    stream: &mut UnixStream,
    limits: LocalIpcLimits,
) -> Result<Vec<u8>> {
    let mut header = [0_u8; LOCAL_AUTH_HEADER_LEN];
    stream.read_exact(&mut header)?;
    if header[..4] != LOCAL_AUTH_MAGIC {
        return Err(invalid_auth_prelude(
            "invalid local IPC authentication prelude magic",
        ));
    }
    let version = u16::from_be_bytes([header[4], header[5]]);
    if version != LOCAL_AUTH_VERSION {
        return Err(invalid_auth_prelude(
            "unsupported local IPC authentication prelude version",
        ));
    }
    if header[6] != 0 || header[7] != 0 {
        return Err(invalid_auth_prelude(
            "unsupported local IPC authentication prelude flags",
        ));
    }
    let evidence_len = u32::from_be_bytes(
        header[8..12]
            .try_into()
            .map_err(|_| invalid_auth_prelude("invalid authentication evidence length"))?,
    );
    if evidence_len > limits.max_auth_evidence_bytes {
        return Err(auth_evidence_too_large());
    }
    let evidence_len = usize::try_from(evidence_len).map_err(|_| auth_evidence_too_large())?;
    let mut evidence = vec![0_u8; evidence_len];
    stream.read_exact(&mut evidence)?;
    Ok(evidence)
}

fn serve_authenticated_stream(
    mut reader: UnixStream,
    connection: &Arc<HostedConnection>,
    wire_limits: WireLimits,
    local_limits: LocalIpcLimits,
    host_in_flight_limit: usize,
) -> Result<()> {
    let worker_count = local_limits
        .max_workers_per_connection
        .min(host_in_flight_limit);
    if worker_count == 0 {
        connection.close();
        return Err(resource_limit(
            "local IPC connection has no request capacity",
        ));
    }
    let mut writer = reader.try_clone()?;
    thread::scope(|scope| -> Result<()> {
        let (response_tx, response_rx) =
            mpsc::sync_channel::<Vec<u8>>(local_limits.max_pending_responses_per_connection);
        let reader_responses = response_tx.clone();
        let writer_connection = Arc::clone(connection);
        scope.spawn(move || {
            while let Ok(response) = response_rx.recv() {
                if writer.write_all(&response).is_err() {
                    writer_connection.close();
                    break;
                }
            }
        });

        let (ready_tx, ready_rx) = mpsc::sync_channel::<usize>(worker_count);
        let mut worker_senders = Vec::with_capacity(worker_count);
        for worker_id in 0..worker_count {
            let (request_tx, request_rx) = mpsc::sync_channel::<Vec<u8>>(1);
            worker_senders.push(request_tx);
            let worker_connection = Arc::clone(connection);
            let worker_responses = response_tx.clone();
            let worker_ready = ready_tx.clone();
            scope.spawn(move || {
                while let Ok(frame) = request_rx.recv() {
                    let request_id = decode_header(&frame[..WIRE_HEADER_LEN], wire_limits)
                        .map_or(0, |header| header.request_id);
                    let response = worker_connection.handle_frame(&frame).or_else(|error| {
                        encode_error_response_frame(
                            request_id,
                            error.code(),
                            error.message(),
                            wire_limits,
                        )
                    });
                    if let Ok(response) = response {
                        if worker_responses.send(response).is_err() {
                            break;
                        }
                    } else {
                        worker_connection.close();
                        break;
                    }
                    if worker_ready.send(worker_id).is_err() {
                        break;
                    }
                }
            });
            ready_tx
                .send(worker_id)
                .map_err(|_| resource_limit("local IPC worker pool failed to initialize"))?;
        }
        drop(ready_tx);
        drop(response_tx);

        let read_result = read_dispatch_loop(
            &mut reader,
            &worker_senders,
            &ready_rx,
            &reader_responses,
            wire_limits,
        );
        connection.close();
        drop(reader_responses);
        drop(worker_senders);
        read_result
    })
}

fn read_dispatch_loop(
    reader: &mut UnixStream,
    workers: &[mpsc::SyncSender<Vec<u8>>],
    ready_workers: &mpsc::Receiver<usize>,
    responses: &mpsc::SyncSender<Vec<u8>>,
    wire_limits: WireLimits,
) -> Result<()> {
    loop {
        let Some(frame) = read_wire_frame(reader, wire_limits)? else {
            return Ok(());
        };
        let header = decode_header(&frame[..WIRE_HEADER_LEN], wire_limits)?;
        let worker_id = match ready_workers.try_recv() {
            Ok(worker_id) => worker_id,
            Err(mpsc::TryRecvError::Empty) => {
                send_resource_limit(responses, header.request_id, wire_limits)?;
                continue;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(resource_limit("local IPC worker pool is unavailable"));
            }
        };
        let Some(worker) = workers.get(worker_id) else {
            return Err(resource_limit("local IPC worker index is invalid"));
        };
        if worker.try_send(frame).is_err() {
            return Err(resource_limit("local IPC worker became unavailable"));
        }
    }
}

fn send_resource_limit(
    responses: &mpsc::SyncSender<Vec<u8>>,
    request_id: u64,
    wire_limits: WireLimits,
) -> Result<()> {
    let response = encode_error_response_frame(
        request_id,
        ProtocolErrorCode::ResourceLimit,
        "local IPC connection reached its request worker limit",
        wire_limits,
    )?;
    match responses.try_send(response) {
        Ok(()) => Ok(()),
        Err(mpsc::TrySendError::Full(_) | mpsc::TrySendError::Disconnected(_)) => Err(
            resource_limit("local IPC response backlog reached its configured limit"),
        ),
    }
}

fn read_wire_frame(stream: &mut impl Read, wire_limits: WireLimits) -> Result<Option<Vec<u8>>> {
    let mut header = [0_u8; WIRE_HEADER_LEN];
    match stream.read(&mut header[..1]) {
        Ok(0) => return Ok(None),
        Ok(1) => {}
        Ok(_) => unreachable!(),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
            return read_wire_frame(stream, wire_limits);
        }
        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    stream.read_exact(&mut header[1..])?;
    let decoded = decode_header(&header, wire_limits)?;
    let payload_len = usize::try_from(decoded.payload_len)
        .map_err(|_| resource_limit("wire payload length does not fit this platform"))?;
    let mut frame = Vec::with_capacity(WIRE_HEADER_LEN + payload_len);
    frame.extend_from_slice(&header);
    let start = frame.len();
    frame.resize(start + payload_len, 0);
    stream.read_exact(&mut frame[start..])?;
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ResetAfter<'a> {
        prefix: &'a [u8],
    }

    impl Read for ResetAfter<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.prefix.is_empty() {
                return Err(io::ErrorKind::ConnectionReset.into());
            }
            self.prefix.read(buffer)
        }
    }

    #[test]
    fn reset_between_frames_is_peer_disconnect() {
        let mut reader = ResetAfter { prefix: &[] };
        assert!(
            read_wire_frame(&mut reader, WireLimits::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reset_during_a_frame_remains_an_io_error() {
        let mut reader = ResetAfter { prefix: b"C" };
        let error = read_wire_frame(&mut reader, WireLimits::default()).unwrap_err();
        assert_eq!(error.code(), crate::LocalIpcErrorCode::Io);
    }
}
