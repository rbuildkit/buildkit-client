# gRPC Tunnel Architecture: BuildKit Session Protocol Deep Dive

## Table of Contents

1. [Overview](#overview)
2. [Architecture Overview](#architecture-overview)
3. [Communication Flow](#communication-flow)
4. [Protocol Layers](#protocol-layers)
5. [DiffCopy Protocol Details](#diffcopy-protocol-details)
6. [Research Process & Key Findings](#research-process--key-findings)
7. [Implementation Challenges](#implementation-challenges)
8. [Critical Patterns & Pitfalls](#critical-patterns--pitfalls)

---

## Overview

The gRPC tunnel is the **most complex component** of the BuildKit client, implementing a complete gRPC server **inside a gRPC stream**. This nested protocol architecture enables BuildKit to call back into the client during the build process to access files, authenticate with registries, and perform other operations.

### Why This Exists

BuildKit's design separates the build orchestration (BuildKit daemon) from resource access (client). When BuildKit needs to:
- Read Dockerfile and build context files
- Authenticate with container registries
- Check session health

...it makes gRPC calls **back to the client** through the session stream, inverting the traditional client-server relationship.

---

## Architecture Overview

### High-Level Structure

```
┌─────────────────────────────────────────────────────────────────┐
│                      BuildKit Client                            │
│                                                                 │
│  ┌─────────────┐         ┌──────────────────────────────────┐ │
│  │   Session   │────────▶│         gRPC Tunnel              │ │
│  │   Manager   │         │                                  │ │
│  └─────────────┘         │  ┌────────────────────────────┐  │ │
│         │                │  │   HTTP/2 Server (h2)       │  │ │
│         │                │  │                            │  │ │
│         │                │  │  ┌──────────────────────┐  │  │ │
│         │                │  │  │ Request Router       │  │  │ │
│         │                │  │  │                      │  │  │ │
│         │                │  │  │ /Health/Check       │  │  │ │
│         │                │  │  │ /FileSync/DiffCopy  │  │  │ │
│         │                │  │  │ /Auth/Credentials   │  │  │ │
│         │                │  │  │ /Auth/FetchToken    │  │  │ │
│         │                │  │  └──────────────────────┘  │  │ │
│         │                │  └────────────────────────────┘  │ │
│         │                └──────────────────────────────────┘ │
│         │                                                      │
│         ▼                                                      │
│  ┌──────────────────┐                                         │
│  │ BytesMessage     │ ◀───────────────────────────────────────┤
│  │ Stream (gRPC)    │                                         │
│  └──────────────────┘                                         │
└─────────────────────────────────────────────────────────────────┘
                      │
                      │ control.Session(stream)
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│                    BuildKit Daemon                              │
│                                                                 │
│  ┌──────────────────┐       ┌──────────────────────────────┐  │
│  │  Solve Engine    │──────▶│  Session Client              │  │
│  │                  │       │                              │  │
│  │  1. Parse        │       │  Makes gRPC calls over       │  │
│  │     Dockerfile   │       │  HTTP/2 tunnel:              │  │
│  │  2. Execute      │       │                              │  │
│  │     build steps  │       │  - DiffCopy (get files)      │  │
│  │  3. Push image   │       │  - Credentials (auth)        │  │
│  │                  │       │  - Health checks             │  │
│  └──────────────────┘       └──────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Communication Flow

### Phase 1: Session Establishment

```rust
// File: src/session/mod.rs:73-152

pub async fn start(&mut self, mut control: ControlClient<Channel>) -> Result<()> {
    // 1. Create bidirectional channel
    let (tx, mut rx) = mpsc::channel::<BytesMessage>(128);

    // 2. Create outbound stream for sending to BuildKit
    let outbound = async_stream::stream! {
        while let Some(msg) = rx.recv().await {
            yield msg;
        }
    };

    // 3. Attach session metadata headers
    let mut request = tonic::Request::new(outbound);
    request.metadata_mut().insert(
        "X-Docker-Expose-Session-Uuid",
        session_id.clone()
    );
    // ... more headers ...

    // 4. Start the bidirectional gRPC session
    let response = control.session(request).await?;
    let mut inbound = response.into_inner();

    // 5. Create HTTP/2 tunnel channels
    let (inbound_tx, inbound_rx) = mpsc::channel(128);
    let (outbound_tx, mut outbound_rx) = mpsc::channel(128);

    // 6. Spawn forwarding tasks
    tokio::spawn(async move {
        // Forward BuildKit → Tunnel
        while let Ok(Some(msg)) = inbound.message().await {
            inbound_tx.send(msg).await;
        }
    });

    tokio::spawn(async move {
        // Forward Tunnel → BuildKit
        while let Some(msg) = outbound_rx.recv().await {
            tx.send(msg).await;
        }
    });

    // 7. Start HTTP/2 server in tunnel
    let tunnel = GrpcTunnel::new(tx.clone(), file_sync, auth);
    tunnel.serve(inbound_rx, outbound_tx).await;
}
```

**Key Concepts:**

1. **BytesMessage Stream**: The outer gRPC stream carries raw bytes
2. **Message Forwarding**: Two tokio tasks forward messages bidirectionally
3. **HTTP/2 Tunnel**: Runs an h2 server over the BytesMessage stream
4. **Session Metadata**: Headers tell BuildKit about this session's capabilities

### Phase 2: HTTP/2 Tunnel Operation

```
Client                          BuildKit
  │                                │
  │  control.Session(stream) ────▶│
  │◀──────────────────────────────│
  │         BytesMessage           │
  │      (HTTP/2 frames)           │
  │                                │
  │         BuildKit starts        │
  │         making gRPC calls      │
  │                                │
  │◀── POST /FileSync/DiffCopy ───│
  │                                │
  │ [HTTP/2 HEADERS frame]         │
  │ method: POST                   │
  │ path: /moby.filesync.v1.       │
  │       FileSync/DiffCopy        │
  │ content-type: application/grpc │
  │                                │
  │◀── [DATA frames with gRPC] ───│
  │    [compressed(1) + length(4)] │
  │    [protobuf payload]          │
  │                                │
  │─── [HEADERS + DATA] ─────────▶│
  │    Response with file data     │
  │                                │
```

---

## Protocol Layers

The communication involves **four nested protocol layers**:

### Layer 1: gRPC (Outer)
```
control.Session(stream<BytesMessage>) → stream<BytesMessage>
```
- **Purpose**: Establish bidirectional communication channel
- **Location**: Between client and BuildKit daemon
- **Protocol**: Standard gRPC streaming

### Layer 2: BytesMessage Encoding
```protobuf
message BytesMessage {
    bytes data = 1;
}
```
- **Purpose**: Wrapper for arbitrary binary data
- **Content**: Raw HTTP/2 frames
- **Location**: `proto/moby/buildkit/v1/control.proto`

### Layer 3: HTTP/2 (Tunneled)
```
HTTP/2 frames inside BytesMessage.data
├── HEADERS frame
├── DATA frames
├── WINDOW_UPDATE frames
└── SETTINGS frames
```
- **Purpose**: Multiplexing multiple gRPC calls over single stream
- **Implementation**: `h2` crate server
- **Location**: `src/session/grpc_tunnel.rs:40-69`

### Layer 4: gRPC (Inner)
```
Each HTTP/2 stream is a gRPC call:
[1 byte compression flag]
[4 bytes message length (big-endian)]
[N bytes protobuf payload]
```
- **Purpose**: Actual service methods (DiffCopy, Credentials, etc.)
- **Format**: Standard gRPC message framing
- **Location**: Handled in `grpc_tunnel.rs:117-135`

---

## DiffCopy Protocol Details

The `DiffCopy` method implements the **fsutil wire protocol** for efficient file transfer. This is a bidirectional streaming RPC with a specific packet exchange sequence.

### Protocol State Machine

```
Client State Machine:

    START
      │
      ├─▶ SENDING_STATS ─────────┐
      │   (Walk directory tree)   │
      │   Send STAT packets       │
      │   for each file/dir       │
      │                           │
      │   Send empty STAT ────────┤
      │   (end of listing)        │
      │                           │
      ▼                           ▼
    WAITING_FOR_REQUESTS    PROCESSING_REQUESTS
      │                           │
      │◀── REQ packet ────────────┤
      │                           │
      ├─▶ Send DATA packets ──────┤
      │   (file content)          │
      │                           │
      │   Send empty DATA ────────┤
      │   (file EOF)              │
      │                           │
      │◀── FIN packet ────────────┤
      │                           │
      ▼                           ▼
    COMPLETE ◀───────────── Send FIN
```

### Packet Types

```protobuf
// From proto/fsutil/types/wire.proto
message Packet {
    PacketType type = 1;
    Stat stat = 2;        // Only for STAT packets
    uint32 id = 3;        // File/directory ID
    bytes data = 4;       // Only for DATA packets
}

enum PacketType {
    PACKET_STAT = 0;  // File/directory metadata
    PACKET_REQ = 1;   // Request file data by ID
    PACKET_DATA = 2;  // File content chunk
    PACKET_FIN = 3;   // End of transfer
    PACKET_ERR = 4;   // Error occurred
}
```

### STAT Packet Structure

```rust
// File: src/session/grpc_tunnel.rs:383-465

async fn collect_and_send_stats(
    root: PathBuf,
    prefix: String,
    send_stream: &mut SendStream<Bytes>,
    id_counter: &mut u32,
) -> Result<HashMap<u32, PathBuf>> {
    let mut file_map = HashMap::new();

    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let name = entry.file_name().to_string_lossy().to_string();
        let rel_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };

        // Assign ID to THIS entry (file or dir)
        let current_id = *id_counter;
        *id_counter += 1;

        // Create STAT packet
        let stat = Stat {
            path: rel_path.clone(),
            mode: if metadata.is_dir() {
                0o040755  // Directory
            } else {
                0o100644  // Regular file
            },
            size: if metadata.is_file() {
                metadata.len() as i64
            } else {
                0
            },
            // ... other fields ...
        };

        let packet = Packet {
            r#type: PacketType::PacketStat as i32,
            stat: Some(stat),
            id: current_id,
            data: vec![],
        };

        send_grpc_packet(send_stream, &packet).await?;

        // Only store files in map (not directories)
        if metadata.is_file() {
            file_map.insert(current_id, entry.path());
        }

        // Recursively process directories
        if metadata.is_dir() {
            let sub_map = collect_and_send_stats(
                entry.path(),
                rel_path,
                send_stream,
                id_counter
            ).await?;
            file_map.extend(sub_map);
        }
    }

    Ok(file_map)
}
```

**Critical Details:**

1. **ID Assignment**: Every entry (file AND directory) gets a unique ID
2. **file_map**: Only stores files (not directories) since only files have data
3. **Root Directory**: The root itself doesn't get a STAT packet, only its children
4. **Mode Bits**: Must be octal: `0o100644` (file), `0o040755` (dir)

### REQ/DATA Exchange

```
BuildKit Request:
┌────────────────────────┐
│ PacketType::REQ        │
│ id: 5                  │  (Request file with ID 5)
│ data: []               │
└────────────────────────┘

Client Response:
┌────────────────────────┐
│ PacketType::DATA       │
│ id: 5                  │
│ data: [chunk 1]        │  (First 32KB chunk)
└────────────────────────┘
┌────────────────────────┐
│ PacketType::DATA       │
│ id: 5                  │
│ data: [chunk 2]        │  (Next chunk)
└────────────────────────┘
┌────────────────────────┐
│ PacketType::DATA       │
│ id: 5                  │
│ data: []               │  (Empty = EOF for this file)
└────────────────────────┘
```

Implementation:

```rust
// File: src/session/grpc_tunnel.rs:467-511

async fn send_file_data_packets(
    file_path: PathBuf,
    id: u32,
    send_stream: &mut SendStream<Bytes>,
) -> Result<()> {
    tracing::info!("Sending file data for: {} (id: {})",
        file_path.display(), id);

    let mut file = tokio::fs::File::open(&file_path).await?;
    let mut buffer = vec![0u8; 32 * 1024]; // 32KB chunks

    loop {
        let n = file.read(&mut buffer).await?;

        if n == 0 {
            // EOF - send empty DATA packet
            let eof_packet = Packet {
                r#type: PacketType::PacketData as i32,
                stat: None,
                id,
                data: vec![],
            };
            send_grpc_packet(send_stream, &eof_packet).await?;
            break;
        }

        // Send data chunk
        let data_packet = Packet {
            r#type: PacketType::PacketData as i32,
            stat: None,
            id,
            data: buffer[..n].to_vec(),
        };
        send_grpc_packet(send_stream, &data_packet).await?;
    }

    Ok(())
}
```

### FIN Handshake

```
BuildKit:                Client:
   │                        │
   │─── REQ(id=1) ────────▶│
   │◀── DATA chunks ───────│
   │                        │
   │─── REQ(id=2) ────────▶│
   │◀── DATA chunks ───────│
   │                        │
   │─── FIN ──────────────▶│  BuildKit: Done requesting
   │                        │
   │◀── FIN ───────────────│  Client: Acknowledged
   │                        │
```

---

## Research Process & Key Findings

### Investigation Timeline

#### 1. Initial Challenge: Understanding the Protocol Nesting

**Problem**: BuildKit documentation doesn't explain how the session stream works internally.

**Discovery Process**:
- Read BuildKit source code: `github.com/moby/buildkit/session`
- Found that BuildKit expects an HTTP/2 connection inside `BytesMessage`
- Key file: `buildkit/session/grpchijack/dial.go` - shows BuildKit's client-side tunnel

**Breakthrough**: Realized we need to implement an HTTP/2 **server**, not client, because BuildKit calls us.

#### 2. gRPC Message Framing

**Problem**: Raw protobuf doesn't work - messages were corrupted.

**Research**:
- gRPC wire format specification: https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
- Every gRPC message has 5-byte prefix:
  ```
  [1 byte: compression flag (0 = none)]
  [4 bytes: message length in big-endian]
  [N bytes: protobuf payload]
  ```

**Implementation** (`grpc_tunnel.rs:127-134`):
```rust
async fn read_unary_request(mut body: h2::RecvStream) -> Result<Bytes> {
    let mut request_data = Vec::new();
    while let Some(chunk) = body.data().await {
        request_data.extend_from_slice(&chunk?);
    }

    // Skip 5-byte gRPC prefix
    let payload = if request_data.len() > 5 {
        Bytes::copy_from_slice(&request_data[5..])
    } else {
        Bytes::new()
    };
    Ok(payload)
}
```

#### 3. DiffCopy Protocol Reverse Engineering

**Problem**: No documentation for fsutil wire protocol packet ordering.

**Research Process**:
1. Read fsutil source: `github.com/tonistiigi/fsutil`
   - `send.go` lines 150-250: Server-side implementation
   - `receive.go`: Client-side implementation

2. Key findings from `send.go:182`:
   ```go
   // Send final empty STAT packet
   if err := ss.Send(&types.Packet{Type: types.PACKET_STAT}); err != nil {
       return err
   }
   ```

3. Discovered the complete sequence:
   - Send all STAT packets (traversing directory tree)
   - Send **empty STAT packet** (no stat field) to signal end of listing
   - Wait for REQ packets
   - Send DATA packets for requested files
   - Send **empty DATA packet** (len=0) for file EOF
   - Send FIN packet when all transfers complete

#### 4. The Nested Loop Bug

**Problem**: DiffCopy would timeout even though all data was sent.

**Root Cause Analysis**:
```rust
// BUGGY CODE:
loop {                              // Outer: read from HTTP/2
    match request_stream.data().await {
        Some(Ok(chunk)) => {
            buffer.extend_from_slice(&chunk);

            while buffer.len() >= 5 {  // Inner: parse messages
                // ... decode packet ...

                if packet_type == PacketType::PacketFin {
                    received_fin = true;
                    break;  // ⚠️ Only breaks inner loop!
                }
            }
            // ❌ Outer loop continues, waits for more data
        }
    }
}
```

**Solution** (`grpc_tunnel.rs:263-345`):
```rust
let mut received_fin = false;

loop {
    match request_stream.data().await {
        Some(Ok(chunk)) => {
            buffer.extend_from_slice(&chunk);

            while buffer.len() >= 5 {
                // ... parse packet ...

                if packet_type == PacketType::PacketFin {
                    received_fin = true;
                    break;  // Exit inner loop
                }
            }

            // ✅ Check flag and exit outer loop
            if received_fin {
                break;
            }
        }
        // ...
    }
}
```

**Lesson**: Nested loops with early exit require explicit flag checking.

#### 5. gRPC Trailers Discovery

**Problem**: Unary RPCs would hang waiting for response completion.

**Research**:
- gRPC spec: Response must end with trailers containing `grpc-status`
- Without trailers, h2 client waits indefinitely

**Solution** (`grpc_tunnel.rs:162-171`):
```rust
// Send response data
send_stream.send_data(Bytes::from(framed), false)?;

// MUST send trailers with grpc-status
let trailers = Response::builder()
    .header("grpc-status", "0")  // 0 = OK
    .body(())
    .unwrap();

send_stream.send_trailers(trailers.headers().clone())?;
```

#### 6. Registry Push Investigation (Current Work)

**Problem**: Images build successfully but don't push to registry.

**Findings**:
1. **BuildKit Configuration**: Registry must be declared in `buildkitd.toml`:
   ```toml
   [registry."registry:5000"]
     http = true  # For HTTP registries
   ```

2. **Mutual Exclusivity**: `http` and `insecure` are mutually exclusive:
   - `http = true`: Plain HTTP connection
   - `insecure = true`: HTTPS with self-signed certs
   - ❌ Don't use both together

3. **Auto-detection Logic** (`solve.rs:131-156`):
   ```rust
   // Extract registry host from tag
   let registry_host = config.tags.first().and_then(|tag| {
       let parts: Vec<&str> = tag.split('/').collect();
       if parts.len() > 1 &&
          (parts[0].contains(':') ||
           parts[0].contains('.') ||
           parts[0] == "localhost") {
           Some(parts[0])
       } else {
           None
       }
   });

   // Auto-detect insecure registries
   if let Some(host) = registry_host {
       let is_insecure = host.starts_with("localhost")
           || host.starts_with("127.0.0.1")
           || host.starts_with("registry:")
           || !host.contains('.');

       if is_insecure {
           export_attrs.insert(
               "registry.insecure".to_string(),
               "true".to_string()
           );
       }
   }
   ```

---

## Implementation Challenges

### Challenge 1: MessageStream - AsyncRead/AsyncWrite Adapter

**Problem**: h2 server expects `AsyncRead + AsyncWrite`, but we have mpsc channels.

**Solution** (`grpc_tunnel.rs:632-720`):
```rust
struct MessageStream {
    inbound_rx: Arc<Mutex<mpsc::Receiver<BytesMessage>>>,
    outbound_tx: mpsc::Sender<BytesMessage>,
    read_buffer: Vec<u8>,
    read_pos: usize,
}

impl AsyncRead for MessageStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Return buffered data first
        if self.read_pos < self.read_buffer.len() {
            let remaining = &self.read_buffer[self.read_pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            self.read_pos += to_copy;
            return Poll::Ready(Ok(()));
        }

        // Poll for next BytesMessage
        let mut rx = self.inbound_rx.try_lock()?;
        match rx.poll_recv(cx) {
            Poll::Ready(Some(msg)) => {
                self.read_buffer = msg.data;
                self.read_pos = 0;
                // Copy to output buffer
                // ...
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
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        // Send BytesMessage through channel
        let msg = BytesMessage {
            data: buf.to_vec(),
        };

        match self.outbound_tx.try_send(msg) {
            Ok(_) => Poll::Ready(Ok(buf.len())),
            Err(_) => Poll::Pending,
        }
    }

    // flush and shutdown implementations...
}
```

### Challenge 2: Service Handler Dispatch

**Problem**: Multiple services (FileSync, Auth, Health) over single tunnel.

**Solution**: Path-based routing (`grpc_tunnel.rs:71-115`):
```rust
async fn handle_request(
    &self,
    req: Request<h2::RecvStream>,
    respond: SendResponse<Bytes>,
) -> Result<()> {
    let method = req.uri().path().to_string();

    match method.as_str() {
        "/grpc.health.v1.Health/Check" => {
            let payload = Self::read_unary_request(body).await?;
            let response = self.handle_health_check(payload).await?;
            self.send_success_response(respond, response).await
        }
        "/moby.filesync.v1.FileSync/DiffCopy" => {
            self.handle_file_sync_diff_copy_stream(body, respond).await
        }
        "/moby.filesync.v1.Auth/Credentials" => {
            let payload = Self::read_unary_request(body).await?;
            let response = self.handle_auth_credentials(payload).await?;
            self.send_success_response(respond, response).await
        }
        _ => {
            self.send_error_response(respond, "Unimplemented").await
        }
    }
}
```

### Challenge 3: Bidirectional Streaming Buffer Management

**Problem**: gRPC messages can span multiple HTTP/2 DATA frames.

**Solution**: Accumulate chunks and parse complete messages (`grpc_tunnel.rs:261-345`):
```rust
let mut buffer = Vec::new();

loop {
    match request_stream.data().await {
        Some(Ok(chunk)) => {
            buffer.extend_from_slice(&chunk);

            // Try to parse complete messages
            while buffer.len() >= 5 {
                // Read gRPC frame header
                let compressed = buffer[0];
                let length = u32::from_be_bytes([
                    buffer[1], buffer[2], buffer[3], buffer[4]
                ]) as usize;

                // Wait for complete message
                if buffer.len() < 5 + length {
                    break;  // Need more data
                }

                // Extract complete message
                let message_data = buffer[5..5+length].to_vec();
                buffer.drain(0..5+length);

                // Decode protobuf
                let packet = Packet::decode(Bytes::from(message_data))?;

                // Process packet...
            }
        }
        Some(Err(e)) => break,
        None => break,
    }
}
```

---

## Critical Patterns & Pitfalls

### ✅ Pattern 1: Always Send gRPC Trailers

```rust
// ❌ BAD: Missing trailers
send_stream.send_data(response, true)?;  // Hangs!

// ✅ GOOD: Explicit trailers
send_stream.send_data(response, false)?;
let trailers = Response::builder()
    .header("grpc-status", "0")
    .body(())
    .unwrap();
send_stream.send_trailers(trailers.headers().clone())?;
```

### ✅ Pattern 2: Empty Packet Signals

```rust
// End of STAT listing
let empty_stat = Packet {
    r#type: PacketType::PacketStat as i32,
    stat: None,  // ← Empty signals end
    id: 0,
    data: vec![],
};

// End of file data
let empty_data = Packet {
    r#type: PacketType::PacketData as i32,
    stat: None,
    id: file_id,
    data: vec![],  // ← Empty signals EOF
};
```

### ⚠️ Pitfall 1: Nested Loop Early Exit

```rust
// ❌ BAD: FIN breaks inner loop only
loop {
    while buffer.len() >= 5 {
        if packet_type == PacketFin {
            break;  // Only exits while!
        }
    }
}

// ✅ GOOD: Flag-based exit
let mut received_fin = false;
loop {
    while buffer.len() >= 5 {
        if packet_type == PacketFin {
            received_fin = true;
            break;
        }
    }
    if received_fin { break; }
}
```

### ⚠️ Pitfall 2: ID Assignment Mismatches

```rust
// ❌ BAD: Only files get IDs
for entry in read_dir(root) {
    if entry.is_file() {
        let id = *id_counter;
        *id_counter += 1;
        send_stat(id, entry);
        file_map.insert(id, path);
    } else {
        // Directory has no ID! BuildKit gets confused.
        send_stat(0, entry);
    }
}

// ✅ GOOD: Every entry gets unique ID
for entry in read_dir(root) {
    let id = *id_counter;
    *id_counter += 1;
    send_stat(id, entry);

    if entry.is_file() {
        file_map.insert(id, path);  // Only files in map
    }
}
```

### ⚠️ Pitfall 3: Mode Bits

```rust
// ❌ BAD: Decimal mode
mode: 755   // BuildKit can't parse this

// ✅ GOOD: Octal mode
mode: if is_dir { 0o040755 } else { 0o100644 }
```

---

## Conclusion

The gRPC tunnel architecture demonstrates several advanced distributed systems patterns:

1. **Protocol Layering**: Four nested protocols working together seamlessly
2. **Role Inversion**: Client acts as server for BuildKit callbacks
3. **Bidirectional Streaming**: Full-duplex communication with proper flow control
4. **Efficient File Transfer**: fsutil protocol minimizes round-trips

### Key Takeaways for Future Developers

1. **Read the Source**: BuildKit and fsutil source code were essential for understanding the protocol
2. **Test Incrementally**: Each protocol layer was tested independently
3. **Buffer Management**: Streaming protocols require careful message boundary detection
4. **Error Handling**: gRPC status codes and trailers are critical for proper cleanup
5. **Nested Loops**: Always use explicit flags for multi-level early exits

### Future Improvements

1. **Performance**: Implement parallel file transfers (multiple DATA streams)
2. **Compression**: Support gRPC compression flag for large files
3. **Caching**: Implement incremental file sync based on checksums
4. **Metrics**: Add instrumentation for transfer rates and packet counts

---

## References

- BuildKit Source: https://github.com/moby/buildkit
- fsutil Source: https://github.com/tonistiigi/fsutil
- gRPC Protocol: https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
- HTTP/2 RFC: https://datatracker.ietf.org/doc/html/rfc7540
- h2 crate docs: https://docs.rs/h2/latest/h2/

---

**Document Version**: 1.0
**Last Updated**: 2025-11-09
**Author**: Claude (AI Assistant) with research and implementation by the BuildKit Rust Client team
