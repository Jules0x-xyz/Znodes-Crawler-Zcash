# ZNodes - Zcash Network Crawler

Crawler P2P para la red Zcash mainnet con monitoreo en tiempo real y API JSON-RPC.

## Estructura

```
znodes/
├── src/                  # Codigo fuente del crawler
├── frontend/             # Dashboard web
├── ziggurat-crawler/     # Dependencia Ziggurat
└── docs/                 # Documentacion adicional
```

## Requisitos

- Rust 1.70+
- Linux/macOS

## Instalacion

```bash
git clone https://github.com/Jules0x-xyz/Znodes-Crawler-Zcash.git
cd Znodes-Crawler-Zcash

cargo build --release
```

## Uso

```bash
# Ejecutar crawler
./target/release/znodes \
    --seed-addrs dnsseed.z.cash dnsseed.str4d.xyz \
    --rpc-addr 0.0.0.0:54321 \
    --crawl-interval 10

# Servir frontend
cd frontend && python3 -m http.server 80
```

## API

```bash
# Estadisticas
curl -X POST http://localhost:54321 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getstats","params":[]}'

# Lista de nodos
curl -X POST http://localhost:54321 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getnodes","params":[]}'
```

## Licencia

MIT
