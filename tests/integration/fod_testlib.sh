#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


fod_test_setup() {
  local root_dir="$1"
  ROOT="${root_dir}"
  POSTGRES_DB="${POSTGRES_DB:-foddbname}"
  POSTGRES_USER="${POSTGRES_USER:-foduser}"
  POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-cichosza}"
  if [[ -z "${FOD_SCHEMA_ADMIN_PASSWORD:-}" ]]; then
    local random_suffix
    random_suffix="$(tr -dc 'A-Za-z0-9_' </dev/urandom | head -c 24 || true)"
    if [[ -z "${random_suffix}" ]]; then
      random_suffix="$(date +%s%N)"
    fi
    FOD_SCHEMA_ADMIN_PASSWORD="fod-${random_suffix}"
  fi
  FOD_CONFIG="${FOD_CONFIG:-${ROOT}/fod_config.ini}"
  FOD_SELINUX="${FOD_SELINUX:-off}"
  FOD_ACL="${FOD_ACL:-off}"
  FOD_DEFAULT_PERMISSIONS="${FOD_DEFAULT_PERMISSIONS:-1}"
  FOD_ALLOW_OTHER="${FOD_ALLOW_OTHER:-0}"
  FOD_ATIME_POLICY="${FOD_ATIME_POLICY:-default}"
  FOD_ROLE="${FOD_ROLE:-auto}"
  FOD_LAZYTIME="${FOD_LAZYTIME:-0}"
  FOD_SYNC="${FOD_SYNC:-0}"
  FOD_DIRSYNC="${FOD_DIRSYNC:-0}"
  FOD_SYNCHRONOUS_COMMIT="${FOD_SYNCHRONOUS_COMMIT:-on}"
  FOD_SELINUX_CONTEXT="${FOD_SELINUX_CONTEXT:-}"
  FOD_SELINUX_FSCONTEXT="${FOD_SELINUX_FSCONTEXT:-}"
  FOD_SELINUX_DEFCONTEXT="${FOD_SELINUX_DEFCONTEXT:-}"
  FOD_SELINUX_ROOTCONTEXT="${FOD_SELINUX_ROOTCONTEXT:-}"
  if [[ -n "${FOD_BOOTSTRAP_BIN:-}" ]]; then
    :
  elif [[ -x "${ROOT}/target/debug/fod-bootstrap" ]]; then
    FOD_BOOTSTRAP_BIN="${ROOT}/target/debug/fod-bootstrap"
  elif [[ -x "${ROOT}/target/release/fod-bootstrap" ]]; then
    FOD_BOOTSTRAP_BIN="${ROOT}/target/release/fod-bootstrap"
  elif [[ -x "${ROOT}/rust_mkfs/target/debug/fod-bootstrap" ]]; then
    FOD_BOOTSTRAP_BIN="${ROOT}/rust_mkfs/target/debug/fod-bootstrap"
  elif [[ -x "${ROOT}/rust_mkfs/target/release/fod-bootstrap" ]]; then
    FOD_BOOTSTRAP_BIN="${ROOT}/rust_mkfs/target/release/fod-bootstrap"
  else
    FOD_BOOTSTRAP_BIN="/usr/local/bin/fod-bootstrap"
  fi
  if [[ -n "${FOD_MKFS_BIN:-}" ]]; then
    :
  elif [[ -x "${ROOT}/target/debug/fod-rust-mkfs" ]]; then
    FOD_MKFS_BIN="${ROOT}/target/debug/fod-rust-mkfs"
  elif [[ -x "${ROOT}/target/release/fod-rust-mkfs" ]]; then
    FOD_MKFS_BIN="${ROOT}/target/release/fod-rust-mkfs"
  elif [[ -x "${ROOT}/rust_mkfs/target/debug/fod-rust-mkfs" ]]; then
    FOD_MKFS_BIN="${ROOT}/rust_mkfs/target/debug/fod-rust-mkfs"
  elif [[ -x "${ROOT}/rust_mkfs/target/release/fod-rust-mkfs" ]]; then
    FOD_MKFS_BIN="${ROOT}/rust_mkfs/target/release/fod-rust-mkfs"
  else
    FOD_MKFS_BIN="/usr/local/bin/fod-rust-mkfs"
  fi
}

fod_test_make_mountpoint() {
  local prefix="$1"
  MOUNTPOINT="$(mktemp -d "${prefix}.XXXXXX")"
  LOG_FILE="$(mktemp "${prefix}.XXXXXX.log")"
  FOD_PID=""
}

fod_test_truthy() {
  [[ "${1:-0}" =~ ^(1|true|True|yes|on)$ ]]
}

fod_test_read_sysfs() {
  local path="$1"
  if [[ -r "${path}" ]]; then
    tr -d '\n' <"${path}" 2>/dev/null || true
  fi
}

fod_test_join_unique_sysfs_values() {
  local pattern="$1"
  local path value
  local -a values=()
  for path in ${pattern}; do
    [[ -r "${path}" ]] || continue
    value="$(fod_test_read_sysfs "${path}")"
    [[ -n "${value}" ]] || continue
    values+=("${value}")
  done
  if ((${#values[@]} == 0)); then
    echo "unknown"
    return
  fi
  printf '%s\n' "${values[@]}" | sort -u | awk '
    BEGIN { first = 1 }
    {
      if (!first) {
        printf ", "
      }
      printf "%s", $0
      first = 0
    }
    END { print "" }
  '
}

fod_test_ac_online_status() {
  local root="/sys/class/power_supply"
  local supply type online seen=0
  [[ -d "${root}" ]] || {
    echo "unknown"
    return
  }
  for supply in "${root}"/*; do
    [[ -e "${supply}" ]] || continue
    type="$(fod_test_read_sysfs "${supply}/type")"
    online="$(fod_test_read_sysfs "${supply}/online")"
    [[ -n "${online}" ]] || continue
    case "${type}" in
      Mains|USB|USB_C|USB_PD|USB_DCP|USB_CDP|USB_ACA)
        seen=1
        if [[ "${online}" == "1" ]]; then
          echo "1"
          return
        fi
        ;;
    esac
  done
  if ((seen)); then
    echo "0"
  else
    echo "unknown"
  fi
}

fod_test_power_metadata() {
  local label="${1:-unknown}"
  local root="/sys/class/power_supply"
  local cpu0="/sys/devices/system/cpu/cpu0/cpufreq"
  local supply name type online status capacity
  local scaling_driver scaling_min_freq scaling_max_freq

  echo "captured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "label=${label}"
  echo "ac_online=$(fod_test_ac_online_status)"
  if [[ -d "${root}" ]]; then
    for supply in "${root}"/*; do
      [[ -e "${supply}" ]] || continue
      name="$(basename "${supply}")"
      type="$(fod_test_read_sysfs "${supply}/type")"
      online="$(fod_test_read_sysfs "${supply}/online")"
      status="$(fod_test_read_sysfs "${supply}/status")"
      capacity="$(fod_test_read_sysfs "${supply}/capacity")"
      printf 'power_supply name=%s' "${name}"
      [[ -n "${type}" ]] && printf ' type=%s' "${type}"
      [[ -n "${online}" ]] && printf ' online=%s' "${online}"
      [[ -n "${status}" ]] && printf ' status=%s' "${status}"
      [[ -n "${capacity}" ]] && printf ' capacity=%s' "${capacity}"
      printf '\n'
    done
  else
    echo "power_supply=unknown"
  fi
  echo "cpu_governors=$(fod_test_join_unique_sysfs_values '/sys/devices/system/cpu/cpu*/cpufreq/scaling_governor')"
  scaling_driver="$(fod_test_read_sysfs "${cpu0}/scaling_driver")"
  scaling_min_freq="$(fod_test_read_sysfs "${cpu0}/scaling_min_freq")"
  scaling_max_freq="$(fod_test_read_sysfs "${cpu0}/scaling_max_freq")"
  echo "cpu_scaling_driver=${scaling_driver:-unknown}"
  echo "cpu_scaling_min_freq_khz=${scaling_min_freq:-unknown}"
  echo "cpu_scaling_max_freq_khz=${scaling_max_freq:-unknown}"
  echo "cpu_energy_performance_preference=$(fod_test_join_unique_sysfs_values '/sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference')"
}

fod_test_log_power_metadata() {
  local label="${1:-unknown}"
  local ac_online
  echo "FOD power metadata (${label}):"
  fod_test_power_metadata "${label}" | sed 's/^/  /'
  ac_online="$(fod_test_ac_online_status)"
  if [[ "${ac_online}" == "0" ]]; then
    echo "WARN: running performance-sensitive FOD test without AC power" >&2
    if fod_test_truthy "${FOD_REQUIRE_AC_POWER:-0}"; then
      echo "FOD_REQUIRE_AC_POWER is set and AC power is not online" >&2
      return 1
    fi
  fi
}

fod_test_write_power_metadata() {
  local output="$1"
  local label="${2:-unknown}"
  mkdir -p "$(dirname "${output}")"
  fod_test_power_metadata "${label}" >"${output}"
}

fod_strace_summary_to_markdown() {
  local summary_file="$1"
  local limit="${FOD_STRACE_SUMMARY_LIMIT:-12}"
  awk -v limit="${limit}" '
    BEGIN {
      print "| % time | seconds | usecs/call | calls | errors | syscall |"
      print "| --- | --- | --- | --- | --- | --- |"
      count = 0
      total = ""
    }
    $6 == "total" {
      total = sprintf("| %s | %s | %s | %s | %s | %s |", $1, $2, $3, $4, $5, $6)
      next
    }
    $1 ~ /^[0-9.]+$/ && count < limit {
      if (NF == 5) {
        printf("| %s | %s | %s | %s |  | %s |\n", $1, $2, $3, $4, $5)
      } else {
        printf("| %s | %s | %s | %s | %s | %s |\n", $1, $2, $3, $4, $5, $6)
      }
      count++
    }
    END {
      if (total != "") {
        print total
      }
    }
  ' "${summary_file}"
}

fod_logical_task_metric() {
  local log_file="$1"
  local operation="$2"
  local metric="$3"
  awk -v expected_operation="${operation}" -v expected_metric="${metric}" '
    /FOD logical task observability:/ && /stage=shutdown/ {
      operation = ""
      value = ""
      for (field_index = 1; field_index <= NF; field_index++) {
        if ($field_index == "operation=" expected_operation) {
          operation = expected_operation
        }
        if (index($field_index, expected_metric "=") == 1) {
          split($field_index, fields, "=")
          value = fields[2]
        }
      }
      if (operation == expected_operation && value != "") {
        selected = value
      }
    }
    END {
      if (selected != "") {
        print selected
      }
    }
  ' "${log_file}"
}

fod_test_cleanup() {
  local restore_errexit=0
  if [[ "$-" == *e* ]]; then
    restore_errexit=1
  fi
  set +e
  if mountpoint -q "${MOUNTPOINT}"; then
    if command -v fusermount3 >/dev/null 2>&1; then
      fusermount3 -u "${MOUNTPOINT}"
    elif command -v fusermount >/dev/null 2>&1; then
      fusermount -u "${MOUNTPOINT}"
    else
      umount "${MOUNTPOINT}"
    fi
  fi
  if [[ -n "${FOD_PID}" ]] && kill -0 "${FOD_PID}" >/dev/null 2>&1; then
    kill "${FOD_PID}" >/dev/null 2>&1 || true
    wait "${FOD_PID}" >/dev/null 2>&1 || true
  fi
  if [[ "${FOD_PROFILE_IO:-0}" =~ ^(1|true|True|yes|on)$ && -f "${LOG_FILE:-}" ]]; then
    local read_calls write_calls copy_file_range_calls
    read_calls="$(grep -oE 'FOD req=[0-9]+ op=read( |$)' "${LOG_FILE}" | sort -u | wc -l | tr -d ' ')"
    write_calls="$(grep -oE 'FOD req=[0-9]+ op=write( |$)' "${LOG_FILE}" | sort -u | wc -l | tr -d ' ')"
    copy_file_range_calls="$(grep -oE 'FOD req=[0-9]+ op=copy_file_range( |$)' "${LOG_FILE}" | sort -u | wc -l | tr -d ' ')"
    if [[ "${read_calls}" == "0" ]]; then
      read_calls="$(fod_logical_task_metric "${LOG_FILE}" file-read admitted_tasks)"
      read_calls="${read_calls:-0}"
    fi
    if [[ "${write_calls}" == "0" ]]; then
      write_calls="$(fod_logical_task_metric "${LOG_FILE}" file-write admitted_tasks)"
      write_calls="${write_calls:-0}"
    fi
    if [[ "${copy_file_range_calls}" == "0" ]]; then
      copy_file_range_calls="$(fod_logical_task_metric "${LOG_FILE}" file-copy admitted_tasks)"
      copy_file_range_calls="${copy_file_range_calls:-0}"
    fi
    printf 'FOD callback counts: read=%s write=%s copy_file_range=%s\n' \
      "${read_calls}" "${write_calls}" "${copy_file_range_calls}"
    echo "FOD boundary profile summary:"
    grep -A40 -E "FOD boundary profile:" "${LOG_FILE}" || tail -n 40 "${LOG_FILE}" || true
  fi
  if [[ -n "${FOD_STRACE_SUMMARY_FILE:-}" && -f "${FOD_STRACE_SUMMARY_FILE}" ]]; then
    echo "FOD strace profile summary${FOD_STRACE_LABEL:+ (${FOD_STRACE_LABEL})}:"
    fod_strace_summary_to_markdown "${FOD_STRACE_SUMMARY_FILE}" || cat "${FOD_STRACE_SUMMARY_FILE}" || true
  fi
  if [[ -n "${FOD_TEST_LOG_ARCHIVE:-}" && -f "${LOG_FILE:-}" ]]; then
    mkdir -p "$(dirname "${FOD_TEST_LOG_ARCHIVE}")"
    cp "${LOG_FILE}" "${FOD_TEST_LOG_ARCHIVE}"
  fi
  rm -rf "${MOUNTPOINT}" "${LOG_FILE}"
  if [[ -n "${FOD_STRACE_SUMMARY_FILE:-}" ]]; then
    rm -f "${FOD_STRACE_SUMMARY_FILE}"
  fi
  MOUNTPOINT=""
  LOG_FILE=""
  FOD_PID=""
  FOD_STRACE_SUMMARY_FILE=""
  FOD_STRACE_LABEL=""
  FOD_TEST_LOG_ARCHIVE=""
  if (( restore_errexit )); then
    set -e
  fi
}

fod_trace_env_prefix() {
  local -n prefix_ref="$1"
  prefix_ref=()
  if [[ -n "${ADMP_TRACE_ENV:-}" ]]; then
    read -r -a prefix_ref <<<"${ADMP_TRACE_ENV}"
    prefix_ref=(env "${prefix_ref[@]}")
  fi
}

fod_test_init_schema() {
  local -a trace_prefix=()
  fod_trace_env_prefix trace_prefix
  local status_output
  status_output="$(
    POSTGRES_DB="${POSTGRES_DB}" POSTGRES_USER="${POSTGRES_USER}" POSTGRES_PASSWORD="${POSTGRES_PASSWORD}" \
      FOD_CONFIG="${FOD_CONFIG}" "${trace_prefix[@]}" "${FOD_MKFS_BIN}" status 2>/dev/null || true
  )"
  if grep -Fq "FOD ready: yes" <<<"${status_output}"; then
    return 0
  fi
  POSTGRES_DB="${POSTGRES_DB}" POSTGRES_USER="${POSTGRES_USER}" POSTGRES_PASSWORD="${POSTGRES_PASSWORD}" \
    FOD_CONFIG="${FOD_CONFIG}" "${trace_prefix[@]}" "${FOD_MKFS_BIN}" init --schema-admin-password "${FOD_SCHEMA_ADMIN_PASSWORD}"
}

fod_test_build_args() {
  FOD_ARGS=(--role "${FOD_ROLE}" --selinux "${FOD_SELINUX}" --acl "${FOD_ACL}" --atime-policy "${FOD_ATIME_POLICY}")

  if [[ "${FOD_DEFAULT_PERMISSIONS}" =~ ^(0|false|False|no)$ ]]; then
    FOD_ARGS+=(--no-default-permissions)
  else
    FOD_ARGS+=(--default-permissions)
  fi

  if [[ "${FOD_LAZYTIME}" =~ ^(0|false|False|no|off)$ ]]; then :; else FOD_ARGS+=(--lazytime); fi
  if [[ "${FOD_SYNC}" =~ ^(0|false|False|no|off)$ ]]; then :; else FOD_ARGS+=(--sync); fi
  if [[ "${FOD_DIRSYNC}" =~ ^(0|false|False|no|off)$ ]]; then :; else FOD_ARGS+=(--dirsync); fi
}

fod_test_start_mount() {
  local mountpoint="$1"
  fod_test_build_args
  mkdir -p "${mountpoint}"
  local start_cmd=("${FOD_BOOTSTRAP_BIN}" --config "${FOD_CONFIG}" "${FOD_ARGS[@]}" -f "${mountpoint}")
  local -a trace_prefix=()
  fod_trace_env_prefix trace_prefix
  if [[ -n "${FOD_STRACE_SUMMARY_FILE:-}" ]]; then
    mkdir -p "$(dirname "${FOD_STRACE_SUMMARY_FILE}")"
    start_cmd=(strace -f -c -o "${FOD_STRACE_SUMMARY_FILE}" "${start_cmd[@]}")
  fi
  POSTGRES_DB="${POSTGRES_DB}" POSTGRES_USER="${POSTGRES_USER}" POSTGRES_PASSWORD="${POSTGRES_PASSWORD}" \
    FOD_CONFIG="${FOD_CONFIG}" FOD_BOOTSTRAP_BIN="${FOD_BOOTSTRAP_BIN}" FOD_USE_RUST_FUSE=1 \
    FOD_USE_FUSE_CONTEXT=1 FOD_ALLOW_OTHER="${FOD_ALLOW_OTHER}" \
    FOD_SELINUX_CONTEXT="${FOD_SELINUX_CONTEXT}" FOD_SELINUX_FSCONTEXT="${FOD_SELINUX_FSCONTEXT}" \
    FOD_SELINUX_DEFCONTEXT="${FOD_SELINUX_DEFCONTEXT}" FOD_SELINUX_ROOTCONTEXT="${FOD_SELINUX_ROOTCONTEXT}" \
    "${trace_prefix[@]}" "${start_cmd[@]}" >"${LOG_FILE}" 2>&1 &
  FOD_PID=$!

  for _ in $(seq 1 60); do
    if mountpoint -q "${mountpoint}"; then
      return 0
    fi
    if ! kill -0 "${FOD_PID}" >/dev/null 2>&1; then
      cat "${LOG_FILE}"
      return 1
    fi
    sleep 1
  done

  if ! mountpoint -q "${mountpoint}"; then
    cat "${LOG_FILE}"
    echo "FOD mount did not become ready"
    return 1
  fi
}

fod_assert_eq() {
  local actual="$1"
  local expected="$2"
  local message="$3"
  if [[ "${actual}" != "${expected}" ]]; then
    echo "${message}: expected=${expected} actual=${actual}"
    return 1
  fi
}

fod_assert_ge() {
  local actual="$1"
  local expected="$2"
  local message="$3"
  if (( actual < expected )); then
    echo "${message}: expected>=${expected} actual=${actual}"
    return 1
  fi
}

fod_assert_nonzero() {
  local actual="$1"
  local message="$2"
  if [[ -z "${actual}" || "${actual}" == "0" ]]; then
    echo "${message}: expected non-zero actual=${actual}"
    return 1
  fi
}

fod_assert_contains() {
  local file_path="$1"
  local needle="$2"
  if ! grep -Fq -- "${needle}" "${file_path}"; then
    echo "missing '${needle}' in ${file_path}"
    return 1
  fi
}

fod_assert_contains_text() {
  local text="$1"
  local needle="$2"
  if ! grep -Fq -- "${needle}" <<<"${text}"; then
    echo "missing '${needle}' in provided text"
    return 1
  fi
}

fod_stat() {
  local path="$1"
  local fmt="$2"
  stat -c "${fmt}" "${path}"
}

fod_ls() {
  local path="$1"
  local output="$2"
  ls -la "${path}" >"${output}"
}

fod_find_sorted() {
  local path="$1"
  local maxdepth="$2"
  local output="$3"
  find "${path}" -maxdepth "${maxdepth}" -print | sort >"${output}"
}
