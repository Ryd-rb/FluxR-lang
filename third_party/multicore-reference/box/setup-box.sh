#!/usr/bin/env bash
# How the benchmark box is set up, before any submission is installed. Run once as root on a fresh Ubuntu 24.04
# from a checkout of this repo:
#
#   sudo bash box/setup-box.sh
#
# Installs PostgreSQL 18 (PGDG) tuned by pg-conf.sh, Nginx 1.30.x (nginx.org stable) with nginx.conf + site.conf,
# the sysctls/limits for many connections in sysctl.conf, user `challenge` and challenge.service.
# Nothing else on the box is tuned. Then load the seed with: sudo -u postgres bash seed/make-seed.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
export DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=a
say() { printf '\n[setup %s] %s\n' "$(date +%T)" "$*"; }
APT="apt-get -y -q -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold"

say "base packages"
$APT update
$APT upgrade
$APT install build-essential pkg-config git curl jq sysstat rsync unzip ca-certificates gnupg lsb-release
CODENAME="$(lsb_release -cs)"
install -d /usr/share/keyrings

say "PostgreSQL 18 from the PGDG apt repo, Nginx stable from nginx.org"
if [ ! -f /etc/apt/sources.list.d/pgdg.list ]; then
  curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor --yes -o /usr/share/keyrings/pgdg.gpg
  echo "deb [signed-by=/usr/share/keyrings/pgdg.gpg] https://apt.postgresql.org/pub/repos/apt $CODENAME-pgdg main" > /etc/apt/sources.list.d/pgdg.list
fi
if [ ! -f /etc/apt/sources.list.d/nginx.list ]; then
  curl -fsSL https://nginx.org/keys/nginx_signing.key | gpg --dearmor --yes -o /usr/share/keyrings/nginx-archive-keyring.gpg
  echo "deb [signed-by=/usr/share/keyrings/nginx-archive-keyring.gpg] https://nginx.org/packages/ubuntu $CODENAME nginx" > /etc/apt/sources.list.d/nginx.list
  printf 'Package: *\nPin: origin nginx.org\nPin: release o=nginx\nPin-Priority: 900\n' > /etc/apt/preferences.d/99nginx
fi
$APT update
$APT install postgresql-18 postgresql-client-18 nginx

say "PostgreSQL: localhost only, scram for 127.0.0.1, pgtune web settings, role conduit"
PGD=/etc/postgresql/18/main
sed -ri "s/^#?listen_addresses\s*=.*/listen_addresses = 'localhost'/" "$PGD/postgresql.conf"
grep -q "^include_dir = 'conf.d'" "$PGD/postgresql.conf" || echo "include_dir = 'conf.d'" >> "$PGD/postgresql.conf"
grep -qE '^host\s+all\s+all\s+127\.0\.0\.1/32\s+scram-sha-256' "$PGD/pg_hba.conf" || \
  echo "host    all             all             127.0.0.1/32            scram-sha-256" >> "$PGD/pg_hba.conf"
bash "$HERE/pg-conf.sh"
systemctl enable postgresql >/dev/null
systemctl restart postgresql@18-main
for _ in $(seq 30); do runuser -u postgres -- pg_isready -q && break; sleep 1; done
runuser -u postgres -- psql -qX -v ON_ERROR_STOP=1 <<'SQL'
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'conduit') THEN
    CREATE ROLE conduit LOGIN PASSWORD 'conduit';
  ELSE
    ALTER ROLE conduit LOGIN PASSWORD 'conduit';
  END IF;
END $$;
SQL

say "Nginx: main config + site (stopped again by the run if a submission serves port 80 directly)"
install -m 644 "$HERE/nginx.conf" /etc/nginx/nginx.conf
rm -f /etc/nginx/conf.d/default.conf
install -m 644 "$HERE/site.conf" /etc/nginx/conf.d/challenge.conf
nginx -t
systemctl enable nginx >/dev/null
systemctl restart nginx

say "kernel / limits for high connection counts"
install -m 644 "$HERE/sysctl.conf" /etc/sysctl.d/90-challenge.conf
if [ -f /proc/sys/net/netfilter/nf_conntrack_max ]; then
  echo "net.netfilter.nf_conntrack_max = 1048576" > /etc/sysctl.d/91-conntrack.conf
fi
sysctl -q --system
install -d /etc/systemd/system.conf.d
printf '[Manager]\nDefaultLimitNOFILE=200000:1048576\n' > /etc/systemd/system.conf.d/90-nofile.conf
printf 'root soft nofile 200000\nroot hard nofile 1048576\n* soft nofile 200000\n* hard nofile 1048576\n' > /etc/security/limits.d/90-nofile.conf
systemctl daemon-reexec

say "user challenge, /opt/challenge, challenge.service"
id challenge >/dev/null 2>&1 || useradd --create-home --shell /bin/bash challenge
install -d -m 755 -o challenge -g challenge /opt/challenge
install -d -m 750 -o root -g challenge /etc/challenge
install -m 644 "$HERE/challenge.service" /etc/systemd/system/challenge.service
systemctl daemon-reload

say "versions"
{
  echo "host $(hostname) $(nproc) vCPU, $(awk '/MemTotal/{print int($2/1024)}' /proc/meminfo) MiB, $(lsb_release -ds), kernel $(uname -r)"
  echo "cpu $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs)"
  echo "postgres $(runuser -u postgres -- psql -tAXc 'show server_version')"
  echo "nginx $(nginx -v 2>&1 | cut -d/ -f2)"
} | tee /root/versions.txt
say "DONE"
