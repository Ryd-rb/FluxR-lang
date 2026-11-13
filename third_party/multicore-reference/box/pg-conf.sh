#!/usr/bin/env bash
# Writes the Postgres tuning include for THIS box from its cores and RAM, using pgtune's "Web application"
# formulas (https://pgtune.leopard.in.ua, DB type "web", storage SSD, max_connections 100).
#
#   pg-conf.sh            # write /etc/postgresql/18/main/conf.d/90-pgtune.conf (restart Postgres afterwards)
#   pg-conf.sh --print    # only print the values
#
# RAM is the box's nominal size: MemTotal rounded to whole GiB (15990 MiB -> 16 GB), which is the number
# anyone would type into pgtune. Cores = nproc.
set -euo pipefail
CORES=$(nproc)
MEM_KB=$(awk '/MemTotal/{print $2}' /proc/meminfo)
RAM_GB=$(( (MEM_KB + 524288) / 1048576 ))
RAM_KB=$(( RAM_GB * 1048576 ))
MAX_CONN=100

SB_KB=$(( RAM_KB / 4 ))                                  # shared_buffers = RAM/4
ECS_KB=$(( RAM_KB * 3 / 4 ))                             # effective_cache_size = 3/4 RAM
MWM_KB=$(( RAM_KB / 16 )); (( MWM_KB > 2097152 )) && MWM_KB=2097152   # maintenance_work_mem = RAM/16, cap 2 GB
PWG=$(( (CORES + 1) / 2 ))                               # ceil(cores/2)
# pgtune: work_mem = (RAM - shared_buffers) / ((max_connections + max_worker_processes) * 3) / max_parallel_workers_per_gather
# (the "web" profile uses this value as is; floor 64 kB)
WM_KB=$(( (RAM_KB - SB_KB) / ((MAX_CONN + CORES) * 3) / PWG )); (( WM_KB < 64 )) && WM_KB=64

conf() {
cat <<CONF
# pgtune "Web application": ${RAM_GB} GB RAM, ${CORES} CPU(s), ${MAX_CONN} connections, SSD. Written by box/pg-conf.sh
max_connections = ${MAX_CONN}
shared_buffers = $(( SB_KB / 1024 ))MB
effective_cache_size = $(( ECS_KB / 1024 ))MB
maintenance_work_mem = $(( MWM_KB / 1024 ))MB
checkpoint_completion_target = 0.9
wal_buffers = 16MB
default_statistics_target = 100
random_page_cost = 1.1
effective_io_concurrency = 200
work_mem = ${WM_KB}kB
huge_pages = try
min_wal_size = 1GB
max_wal_size = 4GB
max_worker_processes = ${CORES}
max_parallel_workers = ${CORES}
max_parallel_workers_per_gather = ${PWG}
max_parallel_maintenance_workers = ${PWG}
CONF
}
if [ "${1:-}" = --print ]; then conf; exit 0; fi
D=/etc/postgresql/18/main/conf.d
mkdir -p "$D"
conf > "$D/90-pgtune.conf"
chown postgres:postgres "$D/90-pgtune.conf"
echo "wrote $D/90-pgtune.conf"; cat "$D/90-pgtune.conf"
