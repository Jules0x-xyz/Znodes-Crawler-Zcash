#!/bin/bash
# Script de diagnóstico para ZNodes

echo "==================================="
echo "   ZNodes - Diagnóstico Crawler"
echo "==================================="
echo ""

# Check if crawler is running
if pgrep -f "target/release/crawler" > /dev/null; then
    echo "✅ Crawler está corriendo"
    CRAWLER_PID=$(pgrep -f "target/release/crawler")
    echo "   PID: $CRAWLER_PID"
else
    echo "❌ Crawler NO está corriendo"
    exit 1
fi

echo ""
echo "📊 Estadísticas Actuales:"
echo "-----------------------------------"

# Get current stats
STATS=$(curl -s -X POST http://localhost:54321 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getnodes","params":[false]}')

TOTAL=$(echo "$STATS" | jq -r '.result.nodes | length')
KNOWN=$(echo "$STATS" | jq -r '.result.stats.num_known_nodes')
CONTACTED=$(echo "$STATS" | jq -r '.result.stats.num_contacted_nodes')
ZCASHD=$(echo "$STATS" | jq -r '.result.stats.num_zcashd_nodes')
ZEBRA=$(echo "$STATS" | jq -r '.result.stats.num_zebra_nodes')
RUNTIME=$(echo "$STATS" | jq -r '.result.stats.crawler_runtime_secs')
TIP=$(echo "$STATS" | jq -r '.result.stats.tip_height_estimate')

echo "Nodos Válidos:    $TOTAL"
echo "  ├─ zcashd:      $ZCASHD"
echo "  └─ Zebra:       $ZEBRA"
echo ""
echo "Nodos Conocidos:  $KNOWN"
echo "Nodos Contactados: $CONTACTED"
echo "Tip Height:       $TIP"
echo ""

# Calculate runtime in human format
if [ "$RUNTIME" -lt 60 ]; then
    RUNTIME_STR="${RUNTIME}s"
elif [ "$RUNTIME" -lt 3600 ]; then
    MINS=$((RUNTIME / 60))
    SECS=$((RUNTIME % 60))
    RUNTIME_STR="${MINS}m ${SECS}s"
else
    HOURS=$((RUNTIME / 3600))
    MINS=$(((RUNTIME % 3600) / 60))
    RUNTIME_STR="${HOURS}h ${MINS}m"
fi

echo "Tiempo corriendo: $RUNTIME_STR"

# Get diagnostics
echo ""
echo "🔍 Diagnóstico de Filtros:"
echo "-----------------------------------"

DIAG=$(curl -s -X POST http://localhost:54321 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getdiagnostics","params":[]}')

NO_UA=$(echo "$DIAG" | jq -r '.result.filtered_by_no_ua')
FLUX=$(echo "$DIAG" | jq -r '.result.filtered_by_flux')
HEIGHT=$(echo "$DIAG" | jq -r '.result.filtered_by_height')
SYNC=$(echo "$DIAG" | jq -r '.result.filtered_by_sync')
PASSED=$(echo "$DIAG" | jq -r '.result.passed_filters')

echo "Filtrados:"
echo "  ├─ Sin User Agent:  $NO_UA"
echo "  ├─ Flux:            $FLUX"
echo "  ├─ Altura baja:     $HEIGHT"
echo "  └─ Desincronizado:  $SYNC"
echo ""
echo "✅ Pasaron filtros:   $PASSED"

# Health check
echo ""
echo "💊 Estado de Salud:"
echo "-----------------------------------"

if [ "$TOTAL" -ge 75 ]; then
    echo "✅ SALUDABLE - $TOTAL nodos (rango normal: 75-120)"
elif [ "$TOTAL" -ge 50 ]; then
    echo "⚠️  ACEPTABLE - $TOTAL nodos (un poco bajo pero OK)"
elif [ "$TOTAL" -ge 30 ]; then
    echo "⚠️  BAJO - $TOTAL nodos (esperar más tiempo o investigar)"
else
    echo "❌ CRÍTICO - Solo $TOTAL nodos (problema serio)"
fi

# Contact rate
CONTACT_RATE=$((CONTACTED * 100 / KNOWN))
echo ""
echo "Tasa de contacto: ${CONTACT_RATE}% ($CONTACTED/$KNOWN)"

if [ "$CONTACT_RATE" -ge 15 ]; then
    echo "✅ Crawler muy activo"
elif [ "$CONTACT_RATE" -ge 10 ]; then
    echo "✅ Crawler activo"
elif [ "$CONTACT_RATE" -ge 5 ]; then
    echo "⚠️  Crawler moderado"
else
    echo "⚠️  Crawler lento (esperar más tiempo)"
fi

echo ""
echo "==================================="
echo "Última actualización: $(date)"
echo "==================================="
