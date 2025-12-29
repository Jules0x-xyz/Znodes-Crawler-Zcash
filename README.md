# ZNodes - Zcash Network Crawler

Crawler P2P para la red Zcash mainnet. Monitoreo en tiempo real con filtrado de red Flux y API JSON-RPC.

![Zcash Logo](zcash-logo.png)

## Estructura del Proyecto

```
znodes/
├── src/                      # Código fuente del crawler
│   ├── main.rs              # Entry point, loops principales
│   ├── protocol.rs          # Protocolo P2P Zcash (handshake, mensajes)
│   ├── network.rs           # Estado de red y conexiones
│   ├── metrics.rs           # Métricas y clasificación de nodos
│   └── rpc.rs               # API JSON-RPC 2.0
├── frontend/                 # Dashboard web
│   ├── index.html           # Interfaz principal (mapa + estadísticas)
│   ├── robots.txt
│   └── sitemap.xml
├── ziggurat-crawler/         # Fork de Ziggurat optimizado
│   ├── src/                 # Código fuente de Ziggurat
│   └── ...
├── docs/                     # Documentación adicional
│   ├── CAMBIOS_ZNODES.md
│   ├── MAPA_ARREGLADO.md
│   ├── MAPA_NITRO.md
│   └── SOLUCION_FRONTEND.md
├── Cargo.toml               # Configuración de Rust
├── Cargo.lock               # Dependencias bloqueadas
├── znodes-diag.sh           # Script de diagnóstico
└── README.md                # Este archivo
```

## Requisitos

- **Rust** 1.70+ (rustup recomendado)
- **Linux/macOS** (probado en Ubuntu 22.04)
- Puerto abierto para RPC (default: 54321)
- Puerto abierto para frontend (default: 80/443)

## Instalacion Rapida

```bash
# Clonar repositorio
git clone https://github.com/Jules0x-xyz/Znodes-Crawler-Zcash.git
cd Znodes-Crawler-Zcash

# Compilar
cargo build --release

# Ejecutar crawler
./target/release/znodes \
    --seed-addrs dnsseed.z.cash dnsseed.str4d.xyz \
    --rpc-addr 0.0.0.0:54321 \
    --crawl-interval 10

# Servir frontend (en otra terminal)
cd frontend
python3 -m http.server 80
# O usar nginx/caddy para produccion
```

## Resumen Ejecutivo

ZNodes es un crawler especializado que mapea la topología de la red Zcash. A diferencia del crawler de ZecHub (basado en Ziggurat, framework de testing), este está optimizado para producción: maneja 2500 conexiones concurrentes, filtra correctamente nodos Flux, y expone métricas limpias via JSON-RPC.

El proyecto surge de la necesidad de tener datos precisos de la red Zcash mainnet. Los crawlers existentes mezclan nodos Flux (fork que usa el mismo protocolo P2P) con nodos Zcash reales, inflando los números. ZNodes implementa un sistema de filtrado multicapa que identifica y excluye Flux, reportando únicamente nodos Zcash legítimos.

**Números actuales:** ~180 nodos Zcash mainnet (75-120 online simultáneamente), filtrados de ~15,000 direcciones descubiertas.

## Comparativa Técnica: ZNodes vs Ziggurat

| Característica | ZNodes | Ziggurat (ZecHub) | Impacto |
|----------------|--------|-------------------|---------|
| **Conexiones concurrentes** | 2,500 | 1,200 | 2x capacidad, mapeo más rápido |
| **Intentos por ciclo** | 1,000 | 500 | Descubrimiento más agresivo |
| **Timeout handshake** | 2,000ms | 300ms | Captura nodos Zebra lentos |
| **Intervalo reconexión** | 60s | 300s | Datos más frescos |
| **Refresh DNS** | Cada 2min | Solo al inicio | Descubre nuevos seeds automático |
| **Manejo errores** | Warn + continúa | Panic + termina | Estabilidad 24/7 |
| **Filtrado Flux** | 4 capas | Básico | Elimina ~2,000 nodos falsos |
| **APIs** | 3 endpoints | 1 endpoint | getmetrics, getstats, getnodes |
| **Priorización** | Nodos sin metadata primero | Aleatorio uniforme | Mapeo más eficiente |

**Resultado:** ZNodes descubre 150 nodos/min vs 40 nodos/min de Ziggurat. Nunca crashea por problemas de red.

## Por Qué Mostramos Menos Nodos que ZecHub

ZecHub reporta ~160 nodos. ZNodes reporta 75-120 nodos. La diferencia está en el filtrado:

### El Problema Flux

La red Flux hizo fork del código de Zcash y opera ~2,000 nodos usando el mismo protocolo P2P. Sin filtrado correcto:

- **ZecHub:** Cuenta todos los nodos MagicBean → mezcla Zcash + Flux
- **ZNodes:** Filtra por versión y user agent → solo Zcash real

**User agents:**
```
/MagicBean:5.4.2/  → Zcash (última versión oficial)
/MagicBean:6.0.0/  → Flux (continuó la numeración)
/Zebra:1.0.0/      → Zcash (cliente alternativo)
```

Zcash se quedó en 5.x. Flux continuó como 6.x+. División clara.

### Sistema de Filtrado (4 Capas)

**1. User Agent Explícito**
Si contiene "flux" → descartado

**2. Verificación de Versión**
MagicBean major >= 6 → es Flux

**3. Validación de Altura**
Altura < 2,500,000 → testnet o muy atrasado

**4. Sincronización (Zebra)**
Zebra > 10,000 bloques atrás → otra red o problema

zcashd puede estar atrasado (sync lento), Zebra no (sync rápido).

### Flujo Real de Filtrado

```
15,000 direcciones descubiertas
  ↓ conectamos
3,500 respondieron handshake (23%)
  ↓ validamos altura
1,200 pasaron check (8%)
  ↓ filtramos Flux
180 nodos Zcash reales (1.2%)
  ↓ online ahora
90-120 activos simultáneos (0.7%)
```

Los 180 son nodos Zcash legítimos. Los 90-120 es el snapshot en tiempo real.

## Arquitectura

**main.rs** - Entry point, loops principales
- Resolución DNS con refresh cada 2min
- Loop de crawling cada 10s (configurable)
- Thread de estadísticas cada 60s
- Servidor RPC con CORS

**protocol.rs** - Protocolo P2P Zcash
- Handshake: VERSION → VERACK
- Procesamiento: ADDR, PING, GETADDR
- Codec de mensajes via pea2pea

**network.rs** - Estado de red
- HashMap con RwLock (muchos lectores, pocos escritores)
- Metadata por nodo: altura, user agent, services
- Limpieza automática de conexiones antiguas

**metrics.rs** - Métricas y clasificación
- Construcción del grafo de conexiones
- Detección de tipos: zcashd, Zebra, Flux, otros
- Estimación del tip (percentil 95)

**rpc.rs** - API JSON-RPC 2.0
- `getmetrics` - NetworkSummary raw (compatible ziggurat)
- `getstats` - Agregados por tipo de cliente
- `getnodes` - Lista filtrada con metadata completa

## Criterios de Clasificación

### zcashd
- User agent contiene "magicbean"
- Versión major < 6
- Altura > 2,500,000

### Zebra
- User agent contiene "zebra"  
- Altura > 2,500,000
- Dentro de 10,000 bloques del tip estimado

### Flux
- User agent contiene "flux" EXPLÍCITAMENTE
- O MagicBean versión >= 6
- Descartado de métricas Zcash

### Estimación del Tip
No usamos el máximo reportado (puede ser error) ni la mediana (nodos atrasados jalan hacia abajo). Usamos **percentil 95**: consenso de nodos sincronizados ignorando outliers.

## Comportamiento del Crawler

### Descubrimiento Inicial (primeros 2min)
1. Resuelve DNS seeds → lista de IPs
2. Conecta secuencialmente a cada seed
3. Handshake: envía VERSION, recibe VERSION + metadata
4. Intercambia VERACK
5. Pide peers con GETADDR
6. Recibe ADDR (hasta 2,500 direcciones)
7. Desconecta y agrega nuevas direcciones

### Loop Continuo
Cada 10 segundos:
1. Separa nodos: sin metadata vs con metadata
2. Baraja ambas listas aleatoriamente
3. Prioriza nodos sin metadata
4. Conecta hasta 1,000 targets
5. Máximo 2,500 conexiones simultáneas
6. Desconecta automático si no responde ADDR en 120s

### Thread de Estadísticas
Cada 60 segundos en paralelo:
1. Snapshot de todos los nodos
2. Construye grafo de conexiones
3. Calcula métricas agregadas
4. Actualiza NetworkSummary
5. Limpia conexiones >10min

## Uso

```bash
cargo build --release

./target/release/znodes \
    --seed-addrs dnsseed.z.cash dnsseed.str4d.xyz \
    --rpc-addr 127.0.0.1:54321 \
    --crawl-interval 10
```

### Consultar RPC

```bash
# Estadísticas agregadas
curl -X POST http://localhost:54321 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getstats","params":[]}'

# Lista de nodos (sin Flux)
curl -X POST http://localhost:54321 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getnodes","params":[false]}'
```

## Preguntas Comunes

**¿Por qué se hacen pasar por zcashd 5.4.2?**
Máxima compatibilidad. Algunos nodos rechazan user agents desconocidos.

**¿Por qué desconectar después de ADDR?**
Solo necesitamos la lista de peers. No bloques ni transacciones. Eficiencia de recursos.

**¿Por qué 2s de timeout vs 300ms?**
Zebra puede tardar 1-2s en responder. Con 300ms perdemos ~30% de nodos Zebra.

**¿Por qué fluctúan los números?**
Capturamos estado real. Nodos van y vienen: wallets que se prenden/apagan, mining pools que reinician, problemas de ISP. Ver 75→110→90 es normal.

**¿Qué es "normal" para Zcash?**
75-120 nodos es el rango esperado. <50 indica problema. 120-150 es spike (nuevo release).

## Licencia

MIT
