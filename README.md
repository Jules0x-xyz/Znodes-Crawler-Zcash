# ZNodes - Zcash Network Monitor

A real-time Zcash network crawler and monitoring dashboard. Discovers and tracks active Zcash nodes (zcashd and Zebra), filtering out Flux nodes and other non-Zcash peers.

## Features

- Real-time node discovery via P2P protocol
- Automatic filtering of Flux and non-Zcash nodes
- Professional web dashboard
- JSON-RPC API for programmatic access
- Export nodes to CSV

## Requirements

- Rust 1.70+
- Git

## Building

```bash
git clone https://github.com/Social-Mask-Labs/znodes.git
cd znodes
cargo build --release
```

## Running

Start the crawler with RPC server:

```bash
cargo run --release -- --seed-addrs dnsseed.z.cash dnsseed.str4d.xyz --rpc-addr 127.0.0.1:54321
```

Then open `frontend/index.html` in your browser.

## Command Line Options

```
OPTIONS:
    -s, --seed-addrs <ADDRS>...     DNS seeds or IP addresses (required)
    -r, --rpc-addr <ADDR>           RPC server address (e.g., 127.0.0.1:54321)
    -c, --crawl-interval <SECS>     Crawl interval in seconds [default: 20]
    -n, --node-listening-port <PORT> Default port [default: 8233]
```

## RPC API

### Get Statistics
```bash
curl -X POST http://127.0.0.1:54321/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":0,"method":"getstats","params":[]}'
```

### Get Nodes
```bash
# Get relevant Zcash nodes only
curl -X POST http://127.0.0.1:54321/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":0,"method":"getnodes","params":[false]}'

# Include Flux and other nodes
curl -X POST http://127.0.0.1:54321/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":0,"method":"getnodes","params":[true]}'
```

## Node Filtering

A node is considered a valid Zcash mainnet node if:

1. Responded to VERSION handshake
2. User agent starts with `/MagicBean` or `/Zebra`
3. User agent does NOT contain "flux"
4. MagicBean version is NOT 6.x (Flux)
5. Block height > 2,500,000 and within ±10,000 of estimated tip

## Project Structure

```
znodes/
├── Cargo.toml          # Rust dependencies
├── main.rs             # Entry point
├── protocol.rs         # P2P protocol handling
├── network.rs          # Network state management
├── metrics.rs          # Network metrics
├── rpc.rs              # JSON-RPC server
└── frontend/
    └── index.html      # Web dashboard
```

## License

MIT
