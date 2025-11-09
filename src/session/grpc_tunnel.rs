//! gRPC tunneling protocol for session stream using h2
//!
//! BuildKit establishes an HTTP/2 connection inside the bidirectional session stream.
//! We use the h2 crate to handle the HTTP/2 server protocol.

use anyhow::{Context, Result};
use bytes::Bytes;
use h2::server::{self, SendResponse};
use http::{Request, Response, StatusCode};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;
use prost::Message as ProstMessage;

use crate::proto::moby::buildkit::v1::BytesMessage;
use super::{FileSyncServer, AuthServer, SecretsServer};

/// Stream multiplexer for handling gRPC tunneled through session
pub struct GrpcTunnel {
    file_sync: Option<FileSyncServer>,
    auth: Option<AuthServer>,
    secrets: Option<SecretsServer>,
}

impl GrpcTunnel {
    /// Create a new gRPC tunnel
    pub fn new(
        _response_tx: mpsc::Sender<BytesMessage>,
        file_sync: Option<FileSyncServer>,
        auth: Option<AuthServer>,
        secrets: Option<SecretsServer>,
    ) -> Self {
        Self {
            file_sync,
            auth,
            secrets,
        }
    }

    /// Start HTTP/2 server over the session stream
    pub async fn serve(
        self,
        inbound_rx: mpsc::Receiver<BytesMessage>,
        outbound_tx: mpsc::Sender<BytesMessage>,
    ) -> Result<()> {
        let tunnel = Arc::new(self);

        // Create a wrapper that implements AsyncRead + AsyncWrite
        let stream = MessageStream::new(inbound_rx, outbound_tx);

        // Start HTTP/2 server
        let mut h2_conn = server::handshake(stream).await
            .context("Failed to complete HTTP/2 handshake")?;

        tracing::info!("HTTP/2 server started in session tunnel");

        // Accept incoming HTTP/2 streams
        while let Some(result) = h2_conn.accept().await {
            let (request, respond) = result.context("Failed to accept HTTP/2 stream")?;
            let tunnel_ref = Arc::clone(&tunnel);

            tokio::spawn(async move {
                if let Err(e) = tunnel_ref.handle_request(request, respond).await {
                    tracing::error!("Failed to handle gRPC request: {}", e);
                }
            });
        }

        Ok(())
    }

    /// Handle a single gRPC request
    async fn handle_request(
        &self,
        req: Request<h2::RecvStream>,
        respond: SendResponse<Bytes>,
    ) -> Result<()> {
        let method = req.uri().path().to_string();
        tracing::info!("Received gRPC call: {}", method);

        let body = req.into_body();

        // Dispatch to appropriate service
        match method.as_str() {
            "/grpc.health.v1.Health/Check" => {
                // Read request body for unary RPC
                let payload = Self::read_unary_request(body).await?;
                let response_payload = self.handle_health_check(payload).await?;
                self.send_success_response(respond, response_payload).await
            }
            "/moby.filesync.v1.FileSync/DiffCopy" => {
                // DiffCopy is a bidirectional streaming RPC - pass the stream
                self.handle_file_sync_diff_copy_stream(body, respond).await
            }
            "/moby.filesync.v1.Auth/GetTokenAuthority" => {
                // Token-based auth not supported - return error to make BuildKit fall back
                // BuildKit requires either a valid pubkey or error to properly fallback to Credentials
                tracing::info!("Auth.GetTokenAuthority called - returning not implemented");
                self.send_error_response(respond, "Token auth not implemented").await
            }
            "/moby.filesync.v1.Auth/Credentials" => {
                let payload = Self::read_unary_request(body).await?;
                let response_payload = self.handle_auth_credentials(payload).await?;
                self.send_success_response(respond, response_payload).await
            }
            "/moby.filesync.v1.Auth/FetchToken" => {
                let payload = Self::read_unary_request(body).await?;
                let response_payload = self.handle_auth_fetch_token(payload).await?;
                self.send_success_response(respond, response_payload).await
            }
            "/moby.buildkit.secrets.v1.Secrets/GetSecret" => {
                let payload = Self::read_unary_request(body).await?;
                let response_payload = self.handle_secrets_get_secret(payload).await?;
                self.send_success_response(respond, response_payload).await
            }
            _ => {
                tracing::warn!("Unknown gRPC method: {}", method);
                self.send_error_response(respond, "Unimplemented").await
            }
        }
    }

    /// Read complete request body for unary RPC
    async fn read_unary_request(mut body: h2::RecvStream) -> Result<Bytes> {
        let mut request_data = Vec::new();

        while let Some(chunk) = body.data().await {
            let chunk = chunk.context("Failed to read request chunk")?;
            request_data.extend_from_slice(&chunk);
            let _ = body.flow_control().release_capacity(chunk.len());
        }

        // Skip the 5-byte gRPC prefix (1 byte compression + 4 bytes length)
        let payload = if request_data.len() > 5 {
            Bytes::copy_from_slice(&request_data[5..])
        } else {
            Bytes::new()
        };

        Ok(payload)
    }

    /// Send successful gRPC response
    async fn send_success_response(
        &self,
        mut respond: SendResponse<Bytes>,
        payload: Bytes,
    ) -> Result<()> {
        // Build gRPC response headers (without grpc-status - that goes in trailers)
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/grpc")
            .body(())
            .unwrap();

        let mut send_stream = respond.send_response(response, false)
            .context("Failed to send response headers")?;

        // Send response with gRPC framing (5-byte prefix)
        let mut framed = Vec::new();
        framed.push(0); // No compression
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(&payload);

        send_stream.send_data(Bytes::from(framed), false)
            .context("Failed to send response data")?;

        // Send trailers with grpc-status
        let trailers = Response::builder()
            .header("grpc-status", "0")
            .body(())
            .unwrap();

        send_stream.send_trailers(trailers.headers().clone())
            .context("Failed to send trailers")?;

        Ok(())
    }

    /// Send error gRPC response
    async fn send_error_response(
        &self,
        mut respond: SendResponse<Bytes>,
        message: &str,
    ) -> Result<()> {
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/grpc")
            .header("grpc-status", "12") // UNIMPLEMENTED
            .header("grpc-message", message)
            .body(())
            .unwrap();

        respond.send_response(response, true)
            .context("Failed to send error response")?;

        Ok(())
    }

    /// Handle FileSync.DiffCopy streaming request
    async fn handle_file_sync_diff_copy_stream(
        &self,
        mut request_stream: h2::RecvStream,
        mut respond: SendResponse<Bytes>,
    ) -> Result<()> {
        use crate::proto::fsutil::types::{Packet, packet::PacketType};
        use prost::Message as ProstMessage;

        tracing::info!("handle_file_sync_diff_copy_stream called");

        let file_sync = match &self.file_sync {
            Some(fs) => fs,
            None => {
                tracing::error!("FileSync not available");
                return self.send_error_response(respond, "FileSync not available").await;
            }
        };

        tracing::info!("FileSync.DiffCopy streaming started");

        // Build response headers
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/grpc")
            .body(())
            .unwrap();

        let mut send_stream = respond.send_response(response, false)
            .context("Failed to send response headers")?;

        tracing::info!("Sent response headers for DiffCopy");

        // Get the root path from FileSyncServer
        let root_path = file_sync.get_root_path();
        tracing::info!("Starting to send directory STAT packets from: {}", root_path.display());

        // First, collect all entries recursively
        let mut entries = Vec::new();
        if let Err(e) = Self::collect_entries_recursive(root_path.clone(), String::new(), &mut entries).await {
            tracing::error!("Error collecting entries: {}", e);
            let trailers = Response::builder()
                .header("grpc-status", "2")
                .header("grpc-message", e.to_string())
                .body(())
                .unwrap();
            let _ = send_stream.send_trailers(trailers.headers().clone());
            return Err(e);
        }

        // Debug: log entries before sorting
        eprintln!("DEBUG: Collected {} entries before sorting", entries.len());
        for (path, _, _) in &entries {
            eprintln!("DEBUG: Entry: {}", path);
        }

        // Sort all entries by their relative path (fsutil requires lexicographic order)
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Debug: log entries after sorting
        eprintln!("DEBUG: Entries after sorting:");
        for (path, _, _) in &entries {
            eprintln!("DEBUG: Sorted: {}", path);
        }

        // Send STAT packets in sorted order and build file map
        use std::collections::HashMap;
        let mut file_map = HashMap::new();
        let mut id_counter = 0u32;
        for (rel_path, entry_path, metadata) in entries {
            let entry_id = id_counter;
            id_counter += 1;

            use crate::proto::fsutil::types::{Packet, packet::PacketType, Stat};

            // Create stat packet
            let mut stat = Stat {
                path: rel_path.clone(),
                mode: 0,
                uid: 0,
                gid: 0,
                size: metadata.len() as i64,
                mod_time: 0,
                linkname: String::new(),
                devmajor: 0,
                devminor: 0,
                xattrs: std::collections::HashMap::new(),
            };

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                stat.mode = metadata.permissions().mode();
            }

            #[cfg(not(unix))]
            {
                stat.mode = if metadata.is_dir() {
                    0o040755  // S_IFDIR | 0o755
                } else {
                    0o100644  // S_IFREG | 0o644
                };
            }

            let mode = stat.mode;
            let stat_packet = Packet {
                r#type: PacketType::PacketStat as i32,
                stat: Some(stat),
                id: entry_id,
                data: vec![],
            };

            // Send stat packet
            tracing::info!("Sending STAT packet for: {} (id: {}, mode: 0o{:o})", rel_path, entry_id, mode);
            if let Err(e) = Self::send_grpc_packet(&mut send_stream, &stat_packet).await {
                tracing::error!("Error sending STAT packet: {}", e);
                let trailers = Response::builder()
                    .header("grpc-status", "2")
                    .header("grpc-message", e.to_string())
                    .body(())
                    .unwrap();
                let _ = send_stream.send_trailers(trailers.headers().clone());
                return Err(e);
            }

            // Store file path in map for later data requests (only for files)
            if metadata.is_file() {
                file_map.insert(entry_id, entry_path);
            }
        }

        // Send final empty STAT packet to indicate end of stats (as done in fsutil send.go line 182)
        let final_stat_packet = Packet {
            r#type: PacketType::PacketStat as i32,
            stat: None,
            id: 0,
            data: vec![],
        };
        Self::send_grpc_packet(&mut send_stream, &final_stat_packet).await
            .context("Failed to send final STAT packet")?;

        tracing::info!("Sent all STAT packets (including final empty STAT), now waiting for REQ packets from BuildKit");

        // Now listen for REQ packets from BuildKit and send the requested files
        // We need to accumulate data across multiple chunks to form complete gRPC messages
        let mut buffer = Vec::new();
        let mut received_fin = false;

        loop {
            // Read next chunk from request stream
            match request_stream.data().await {
                Some(Ok(chunk)) => {
                    buffer.extend_from_slice(&chunk);
                    let _ = request_stream.flow_control().release_capacity(chunk.len());

                    // Try to parse complete gRPC messages from buffer
                    while buffer.len() >= 5 {
                        // Read gRPC frame header (5 bytes)
                        let compressed = buffer[0];
                        let length = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) as usize;

                        if buffer.len() < 5 + length {
                            // Not enough data for complete message yet
                            break;
                        }

                        // Extract the complete message
                        let message_data = buffer[5..5+length].to_vec();
                        buffer.drain(0..5+length);

                        if compressed != 0 {
                            tracing::warn!("Received compressed message, skipping");
                            continue;
                        }

                        // Decode the packet
                        let packet = match Packet::decode(Bytes::from(message_data)) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::error!("Failed to decode packet: {}", e);
                                continue;
                            }
                        };

                        let packet_type = PacketType::try_from(packet.r#type).unwrap_or(PacketType::PacketStat);
                        tracing::debug!("Received packet type: {:?}, id: {}, has_stat: {}",
                            packet_type, packet.id, packet.stat.is_some());

                        match packet_type {
                            PacketType::PacketReq => {
                                // BuildKit is requesting file data for a specific ID
                                tracing::info!("Received REQ packet with id: {}", packet.id);

                                if let Some(file_path) = file_map.get(&packet.id) {
                                    tracing::info!("Sending file data for id {}: {}", packet.id, file_path.display());
                                    if let Err(e) = Self::send_file_data_packets(file_path.clone(), packet.id, &mut send_stream).await {
                                        tracing::error!("Failed to send file data: {}", e);
                                    }
                                } else {
                                    tracing::warn!("File ID {} not found in map (probably a directory, ignoring)", packet.id);
                                }
                            }
                            PacketType::PacketFin => {
                                // BuildKit is signaling it's done requesting files
                                tracing::info!("Received FIN packet from BuildKit, ending transfer");
                                received_fin = true;
                                break;
                            }
                            _ => {
                                tracing::debug!("Ignoring packet type: {:?}", packet_type);
                            }
                        }
                    }

                    // Check if we received FIN and should exit the outer loop
                    if received_fin {
                        break;
                    }
                }
                Some(Err(e)) => {
                    tracing::error!("Error reading request stream: {}", e);
                    break;
                }
                None => {
                    tracing::info!("Request stream ended");
                    break;
                }
            }
        }

        tracing::info!("DiffCopy completed, sending FIN packet");

        // Send FIN packet to indicate all transfers are complete
        let fin_packet = Packet {
            r#type: PacketType::PacketFin as i32,
            stat: None,
            id: 0,
            data: vec![],
        };

        Self::send_grpc_packet(&mut send_stream, &fin_packet).await?;
        tracing::debug!("Sent final FIN packet");

        // Send success trailers
        let trailers = Response::builder()
            .header("grpc-status", "0")
            .body(())
            .unwrap();

        send_stream.send_trailers(trailers.headers().clone())
            .context("Failed to send trailers")?;

        Ok(())
    }

    /// Recursively collect all entries (files and directories) from a path
    /// Returns a vector of (relative_path, absolute_path, metadata) tuples
    fn collect_entries_recursive<'a>(
        path: std::path::PathBuf,
        prefix: String,
        result: &'a mut Vec<(String, std::path::PathBuf, std::fs::Metadata)>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tracing::debug!("Collecting entries from: {} (prefix: {})", path.display(), prefix);

            let mut entries = tokio::fs::read_dir(&path).await
                .with_context(|| format!("Failed to read directory {}", path.display()))?;

            while let Some(entry) = entries.next_entry().await? {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                let rel_path = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{}/{}", prefix, name)
                };

                let entry_path = entry.path();
                let metadata = entry.metadata().await?;

                // Add this entry to result
                result.push((rel_path.clone(), entry_path.clone(), metadata.clone()));

                // Recursively handle directories
                if metadata.is_dir() {
                    Self::collect_entries_recursive(entry_path, rel_path, result).await?;
                }
            }

            Ok(())
        })
    }

    /// Send file data as DATA packets in response to a REQ
    async fn send_file_data_packets(
        path: std::path::PathBuf,
        req_id: u32,
        stream: &mut h2::SendStream<Bytes>,
    ) -> Result<()> {
        use crate::proto::fsutil::types::{Packet, packet::PacketType};
        use tokio::io::AsyncReadExt;

        tracing::info!("Sending file data for: {} (id: {})", path.display(), req_id);

        let mut file = tokio::fs::File::open(&path).await
            .with_context(|| format!("Failed to open file {}", path.display()))?;

        let mut buffer = vec![0u8; 32 * 1024]; // 32KB chunks

        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }

            let data_packet = Packet {
                r#type: PacketType::PacketData as i32,
                stat: None,
                id: req_id,
                data: buffer[..n].to_vec(),
            };

            Self::send_grpc_packet(stream, &data_packet).await?;
        }

        // Send empty DATA packet to indicate end of this file
        // (NOT a FIN packet - FIN is sent only at the very end of all transfers)
        let eof_packet = Packet {
            r#type: PacketType::PacketData as i32,
            stat: None,
            id: req_id,
            data: vec![],
        };

        Self::send_grpc_packet(stream, &eof_packet).await?;
        tracing::debug!("Sent EOF (empty DATA) packet for id: {}", req_id);

        Ok(())
    }

    /// Send a single gRPC-framed packet
    async fn send_grpc_packet(
        stream: &mut h2::SendStream<Bytes>,
        packet: &crate::proto::fsutil::types::Packet,
    ) -> Result<()> {
        use prost::Message as ProstMessage;
        use crate::proto::fsutil::types::packet::PacketType;

        let mut payload = Vec::new();
        packet.encode(&mut payload)?;

        // Add gRPC framing (5-byte prefix)
        let mut framed = Vec::new();
        framed.push(0); // No compression
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(&payload);

        let packet_type = PacketType::try_from(packet.r#type).ok();
        tracing::trace!("Sending packet: type={:?}, id={}, data_len={}, total_frame_len={}",
            packet_type, packet.id, packet.data.len(), framed.len());

        stream.send_data(Bytes::from(framed), false)
            .context("Failed to send packet data")?;

        // Give the h2 stream a chance to flush
        tokio::task::yield_now().await;

        Ok(())
    }

    /// Handle Auth.GetTokenAuthority request
    ///
    /// # Implementation Strategy
    ///
    /// This function implements the **empty response strategy** for GetTokenAuthority,
    /// returning a success response with an empty `public_key` field.
    ///
    /// However, it is currently **unused** in favor of the **error response strategy**
    /// (see line 97-102), which directly returns a gRPC error to BuildKit.
    ///
    /// ## Two Valid Approaches
    ///
    /// Both strategies achieve the same result - BuildKit falls back to `Credentials` auth:
    ///
    /// 1. **Error Response Strategy** (current implementation):
    ///    - Returns `grpc-status: 2` (UNKNOWN/UNIMPLEMENTED)
    ///    - Semantically clearer: "feature not supported"
    ///    - BuildKit detects error → falls back to `Credentials`
    ///
    /// 2. **Empty Response Strategy** (this function):
    ///    - Returns `grpc-status: 0` (OK) with `public_key: []`
    ///    - BuildKit checks `pubKey == nil` → falls back to `Credentials`
    ///    - See BuildKit source: `util/resolver/authorizer.go:183-190`
    ///
    /// ## Why Keep This Function?
    ///
    /// Preserved for future use if switching to empty response strategy is desired:
    /// - To avoid error logs in BuildKit output
    /// - To maintain protocol compatibility with certain BuildKit versions
    /// - As reference implementation for token authority protocol
    ///
    /// ## How to Use This Function
    ///
    /// To switch from error response to empty response strategy, replace lines 97-102 with:
    ///
    /// ```rust
    /// "/moby.filesync.v1.Auth/GetTokenAuthority" => {
    ///     let payload = Self::read_unary_request(body).await?;
    ///     let response_payload = self.handle_auth_get_token_authority(payload).await?;
    ///     self.send_success_response(respond, response_payload).await
    /// }
    /// ```
    #[allow(dead_code)]
    async fn handle_auth_get_token_authority(&self, payload: Bytes) -> Result<Bytes> {
        use crate::proto::moby::filesync::v1::{GetTokenAuthorityRequest, GetTokenAuthorityResponse};

        let request = GetTokenAuthorityRequest::decode(payload)
            .context("Failed to decode GetTokenAuthorityRequest")?;

        tracing::info!("Auth.GetTokenAuthority request for host: {}", request.host);

        // Return empty response - we don't implement token-based auth
        // BuildKit will detect empty public_key and fall back to Credentials method
        let response = GetTokenAuthorityResponse {
            public_key: vec![],
        };

        let mut buf = Vec::new();
        response.encode(&mut buf)?;
        Ok(Bytes::from(buf))
    }

    /// Handle Auth.Credentials request
    async fn handle_auth_credentials(&self, payload: Bytes) -> Result<Bytes> {
        use crate::proto::moby::filesync::v1::CredentialsRequest;
        use tonic::Request;
        use crate::proto::moby::filesync::v1::auth_server::Auth;

        let request = CredentialsRequest::decode(payload)
            .context("Failed to decode CredentialsRequest")?;

        tracing::info!("Auth.Credentials request for host: {}", request.host);

        // Use AuthServer if configured, otherwise return empty credentials
        let response = if let Some(auth) = &self.auth {
            match auth.credentials(Request::new(request.clone())).await {
                Ok(resp) => {
                    let inner = resp.into_inner();
                    if !inner.username.is_empty() {
                        tracing::debug!("Returning credentials for host: {} (username: {})",
                            request.host, inner.username);
                    } else {
                        tracing::debug!("No credentials found for host: {}, returning empty", request.host);
                    }
                    inner
                }
                Err(status) => {
                    tracing::warn!("Failed to get credentials: {}, returning empty", status.message());
                    use crate::proto::moby::filesync::v1::CredentialsResponse;
                    CredentialsResponse {
                        username: String::new(),
                        secret: String::new(),
                    }
                }
            }
        } else {
            tracing::debug!("No auth configured, returning empty credentials");
            use crate::proto::moby::filesync::v1::CredentialsResponse;
            CredentialsResponse {
                username: String::new(),
                secret: String::new(),
            }
        };

        let mut buf = Vec::new();
        response.encode(&mut buf)?;
        Ok(Bytes::from(buf))
    }

    /// Handle Auth.FetchToken request
    async fn handle_auth_fetch_token(&self, _payload: Bytes) -> Result<Bytes> {
        use crate::proto::moby::filesync::v1::FetchTokenResponse;

        tracing::info!("Auth.FetchToken called");

        let response = FetchTokenResponse {
            token: String::new(),
            expires_in: 0,
            issued_at: 0,
        };

        let mut buf = Vec::new();
        response.encode(&mut buf)?;
        Ok(Bytes::from(buf))
    }

    /// Handle Secrets.GetSecret request
    async fn handle_secrets_get_secret(&self, payload: Bytes) -> Result<Bytes> {
        use crate::proto::moby::secrets::v1::GetSecretRequest;

        let request = GetSecretRequest::decode(payload)
            .context("Failed to decode GetSecretRequest")?;

        tracing::info!("Secrets.GetSecret request for ID: {}", request.id);

        // If secrets service is not configured, return empty data
        let response = if let Some(secrets) = &self.secrets {
            // Use the SecretsServer's get_secret implementation through the Secrets trait
            use tonic::Request;
            use crate::proto::moby::secrets::v1::secrets_server::Secrets;

            match secrets.get_secret(Request::new(request.clone())).await {
                Ok(resp) => {
                    let inner = resp.into_inner();
                    tracing::debug!("Returning secret '{}' ({} bytes)", request.id, inner.data.len());
                    inner
                }
                Err(status) => {
                    tracing::warn!("Secret '{}' not found: {}", request.id, status.message());
                    return Err(anyhow::anyhow!("Secret not found: {}", status.message()));
                }
            }
        } else {
            tracing::warn!("Secrets service not configured");
            return Err(anyhow::anyhow!("Secrets service not configured"));
        };

        let mut buf = Vec::new();
        response.encode(&mut buf)?;
        Ok(Bytes::from(buf))
    }

    /// Handle Health.Check request
    async fn handle_health_check(&self, _payload: Bytes) -> Result<Bytes> {
        tracing::info!("Health check called");

        // Health check response: status = SERVING (1)
        // The proto definition is:
        // message HealthCheckResponse {
        //   enum ServingStatus {
        //     UNKNOWN = 0;
        //     SERVING = 1;
        //     NOT_SERVING = 2;
        //   }
        //   ServingStatus status = 1;
        // }

        // Manually encode: field 1, varint type, value 1
        let response = vec![0x08, 0x01]; // field 1 (0x08 = 0001|000) = value 1
        Ok(Bytes::from(response))
    }
}

/// A stream that wraps BytesMessage channels to implement AsyncRead + AsyncWrite
struct MessageStream {
    inbound_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<BytesMessage>>>,
    outbound_tx: mpsc::Sender<BytesMessage>,
    read_buffer: Vec<u8>,
    read_pos: usize,
}

impl MessageStream {
    fn new(
        inbound_rx: mpsc::Receiver<BytesMessage>,
        outbound_tx: mpsc::Sender<BytesMessage>,
    ) -> Self {
        Self {
            inbound_rx: Arc::new(tokio::sync::Mutex::new(inbound_rx)),
            outbound_tx,
            read_buffer: Vec::new(),
            read_pos: 0,
        }
    }
}

impl AsyncRead for MessageStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // If we have buffered data, return it
        if self.read_pos < self.read_buffer.len() {
            let remaining = &self.read_buffer[self.read_pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            self.read_pos += to_copy;

            // Clear buffer if fully consumed
            if self.read_pos >= self.read_buffer.len() {
                self.read_buffer.clear();
                self.read_pos = 0;
            }

            return Poll::Ready(Ok(()));
        }

        // Try to receive next message
        let inbound_rx = self.inbound_rx.clone();
        let mut rx = match inbound_rx.try_lock() {
            Ok(rx) => rx,
            Err(_) => return Poll::Pending,
        };

        match rx.poll_recv(cx) {
            Poll::Ready(Some(msg)) => {
                self.read_buffer = msg.data;
                self.read_pos = 0;

                let to_copy = self.read_buffer.len().min(buf.remaining());
                buf.put_slice(&self.read_buffer[..to_copy]);
                self.read_pos = to_copy;

                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Ok(())), // EOF
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for MessageStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let msg = BytesMessage {
            data: buf.to_vec(),
        };

        // Try to send immediately (non-blocking)
        match self.outbound_tx.try_send(msg) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Channel is full, would block
                Poll::Pending
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "Channel closed",
                )))
            }
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
