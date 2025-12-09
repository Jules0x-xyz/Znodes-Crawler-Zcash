# ZNodes

Hellooo! Thankiu for stopping by. This is **ZNodes**, a P2P crawler for the Zcash mainnet. Actually, I created this because I needed accurate real-time monitoring of the network, and this tool helps me a lot to get clean data by filtering out Flux nodes.

## What is this?

ZNodes is a crawler optimized for production that maps the Zcash network topology. By the way, I was checking other tools like ZecHub's crawler and realized they often mix Flux nodes (which use the same P2P protocol) with real Zcash nodes. Am I wrong, or does that make the data less useful? 

So, this project solves that! We currently find about **180 real Zcash nodes** (with 75-120 online simultaneously) out of thousands of addresses, because we strictly filter out the Flux ones. We still have a lot of work to do, but having precise numbers is a great start.

## Key Differences

| Feature | ZNodes | Others (like ZecHub) | Why it matters |
|---------|--------|-------------------|----------------|
| **Connections** | 2,500 | ~1,200 | We can map the network faster. |
| **Flux Filtering** | 4 Layers | Basic | We don't count ~2,000 fake nodes. |
| **Stability** | Warns & Continues | Panics | It runs 24/7 without crashing. |
| **DNS Refresh** | Every 2 min | Once at start | We find new seeds automatically. |

## The Flux Problem (and how we fix it)

Flux forked Zcash code, so they look very similar on the network. But actually, they are different!
- **ZecHub** counts everything.
- **ZNodes** filters by User Agent and Version.

If we see `/MagicBean:6.0.0/` or "flux", we know it's not Zcash (which is on 5.x). This filtering helps me a lot to ensure we are only looking at the **real** Zcash network.

## How to use it

You can run it easily with these commands:

```bash
cargo build --release

./target/release/znodes \
    --seed-addrs dnsseed.z.cash dnsseed.str4d.xyz \
    --rpc-addr 127.0.0.1:54321 \
    --crawl-interval 10
```

### Check the stats

```bash
# Get aggregated stats
curl -X POST http://localhost:54321 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getstats","params":[]}'
```

Thankiu for reading! If you have questions, let me know.

---
This was created with artificial intelligence and a representative of our team in order to better understand the idea and the form of development and be able to reflect it in English.
