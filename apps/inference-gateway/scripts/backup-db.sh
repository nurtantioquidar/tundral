#!/bin/sh
set -eu

umask 077
database_path="${DATABASE_PATH:-./gateway.db}"
backup_dir="${BACKUP_DIR:-./backups}"
retention_days="${BACKUP_RETENTION_DAYS:-14}"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
destination="${backup_dir}/gateway-${timestamp}.db"

mkdir -p "${backup_dir}"
sqlite3 "${database_path}" ".timeout 5000" ".backup '${destination}'"
integrity="$(sqlite3 "${destination}" 'PRAGMA integrity_check;')"
if [ "${integrity}" != "ok" ]; then
  echo "backup integrity check failed" >&2
  exit 1
fi

find "${backup_dir}" -type f -name 'gateway-*.db' -mtime "+${retention_days}" -delete
echo "${destination}"

