#!/bin/bash
# SPDX-License-Identifier: BSD-3-Clause
#
# Hardware-in-the-loop (HIL) integration test steps.
#
# Each step is registered as its own meson test in the "hil" suite (see
# tests/meson.build) and invoked as:
#
#   test_hil.sh --builddir <dir> --step <name>
#
# The steps drive the built qdl binary against a real device in EDL mode, so
# they are not meant for unattended CI. Run them explicitly with:
#
#   QDL_HIL_BUILD=/path/to/build QDL_HIL_STORAGE=ufs \
#       meson test -C build --suite hil
#
# When the required environment is not set a step exits 77, which meson
# reports as SKIP, so a plain "meson test" stays green without hardware.
#
# Required environment:
#   QDL_HIL_BUILD     Build to flash. Auto-detected: a directory uses classic
#                     flashing (prog_firehose_*.elf + rawprogram*.xml +
#                     patch*.xml); a file (.json/.xml/.zip) uses "qdl flash".
#   QDL_HIL_STORAGE   Target storage type: emmc|nand|nvme|spinor|ufs.
#
# Optional environment:
#   QDL_HIL_PROGRAMMER  Firehose programmer .elf (needed by the read/write/
#                       erase/sha256/reset steps). Defaults to the first
#                       prog_firehose_*.elf next to QDL_HIL_BUILD.
#   QDL_HIL_PARTITION   Partition for the read/erase/write steps. The EFI
#                       system partition is small and quick to read and write,
#                       so it defaults to "efi".
#   QDL_HIL_IMAGE       Image the erase/write steps restore. Defaults to the
#                       data the read step reads back, so erase/write
#                       round-trip the partition's own content.
#   QDL_HIL_SERIAL      -S serial, to select one of several attached devices.
#   QDL_HIL_ENTER_EDL_CMD  Command run before the suite's first step to put
#                       the board into EDL mode (e.g. "adb reboot edl" or a
#                       USB relay toggle), for unattended/CI runs. When
#                       unset the board must already be in EDL mode.
#   QDL_HIL_SETTLE      Seconds the list step waits for the device to
#                       enumerate after the enter-EDL command (default 30).
#   QDL_HIL_DEBUG       When set, run every qdl invocation with --debug so
#                       failures leave a full protocol log in the meson
#                       testlog (and the uploaded CI artifacts).
#   QDL_HIL_CONTENTS    contents.xml (optionally with ::specifier) for the
#                       create-zip step; when unset that step is skipped.
#   QDL_HIL_SPARSE      flashmap/contents/zip that flashes sparse images, for
#                       the sparse-flash step; when unset that step is skipped.
#
# Every step runs with --skip-reset to keep the programmer alive across
# invocations; only the final step ends the session with the reset verb.
#
# The nbd step drives the nbdkit plugin instead of qdl. The plugin uploads
# the programmer itself and resets the board when nbdkit exits, so the step
# needs a device that has not been programmed yet and ends the session. It
# lives in its own "hil-nbd" suite, run against a freshly entered EDL mode:
#
#   QDL_HIL_BUILD=... QDL_HIL_STORAGE=... meson test -C build --suite hil-nbd
#
# It needs a build with the plugin (nbdkit-plugin-dev) and the nbdkit,
# nbdinfo and nbdcopy tools, and the suite skips otherwise, before entering
# EDL mode. Besides reading, it writes the primary GPT of physical
# partition 0 back with the bytes it has just read from it. A failed run
# keeps those bytes and the nbdkit log in build/meson-logs/hil-nbd.*.

set -e

SKIP=77

while [[ $# -gt 0 ]]; do
    case "$1" in
        --builddir) builddir="$2"; shift 2 ;;
        --step)     step="$2"; shift 2 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

[[ -n "${builddir}" ]] || { echo "Error: --builddir is required." >&2; exit 1; }
[[ -n "${step}" ]]     || { echo "Error: --step is required." >&2; exit 1; }

uname_out="$(uname -s)"
case "${uname_out}" in
    Linux*|Darwin*) QDL=qdl ;;
    CYGWIN*|MINGW*|MSYS*) QDL=qdl.exe ;;
    *) exit 1 ;;
esac
QEXE="${builddir}/${QDL}"

shopt -s nullglob

# --- fail-fast across the (serialized) suite ------------------------------
# meson runs each step as its own process, so persist a sentinel: the first
# step starts a fresh run, any real failure raises it, and every later step
# bails out instead of touching the device once it is set. A skipped step
# (exit 77) does not count as a failure.
ABORT_FILE="${builddir}/meson-logs/hil.aborted"
# Keep all HIL scratch (partition readbacks, temporary reads) on the build
# disk, never in /tmp - which is commonly a small tmpfs that a full-partition
# readback would exhaust.
HIL_TMPDIR="${builddir}/hil-tmp"
mkdir -p "${HIL_TMPDIR}"
# Readback of the target partition captured by the read step and written back
# after the erase step, so erase/write restore the exact original bytes.
SAVED_IMAGE="${HIL_TMPDIR}/readback.img"

# Set by the nbd step while it holds the original GPT area, so a failure
# can point at the copy needed to restore the board.
NBD_KEEP=""

on_exit() {
    local rc=$?
    if [[ ${rc} -ne 0 && ${rc} -ne ${SKIP} ]]; then
        mkdir -p "$(dirname "${ABORT_FILE}")" 2>/dev/null || true
        : > "${ABORT_FILE}"
        [[ -z "${NBD_KEEP}" ]] ||
            echo "original GPT area (area.img) and nbdkit log kept in ${NBD_KEEP}" >&2
    fi
}
trap on_exit EXIT

# The suite's first step (highest meson priority): "list", or "nbd_list" in
# the hil-nbd suite. Suite-level setup - resetting the fail-fast sentinel,
# entering EDL mode - must anchor to it: doing either in a later step breaks
# the steps running before it. Keep this in sync with the priorities in
# tests/meson.build.
FIRST_STEP=false
[[ "${step}" == list || "${step}" == nbd_list ]] && FIRST_STEP=true

if ${FIRST_STEP}; then
    rm -f "${ABORT_FILE}"
    rm -rf "${HIL_TMPDIR:?}"/*
fi

# --- resolve configuration from the environment ---------------------------
BUILD="${QDL_HIL_BUILD}"
STORAGE="${QDL_HIL_STORAGE}"
PARTITION="${QDL_HIL_PARTITION:-efi}"
SERIAL="${QDL_HIL_SERIAL}"
PROGRAMMER="${QDL_HIL_PROGRAMMER}"
IMAGE="${QDL_HIL_IMAGE}"

if [[ -z "${BUILD}" || -z "${STORAGE}" ]]; then
    echo "set QDL_HIL_BUILD and QDL_HIL_STORAGE to run the HIL suite" >&2
    exit ${SKIP}
fi

# Fail-fast: if an earlier step already failed, stop the whole suite. No
# step is exempt - the first step cleared the sentinel above before this
# check, so a set sentinel always means a failure in this run.
if [[ -e "${ABORT_FILE}" ]]; then
    echo "an earlier HIL step failed; stopping the suite" >&2
    exit ${SKIP}
fi

IS_DIR=false
if [[ -d "${BUILD}" ]]; then
    IS_DIR=true
    BUILDDIR="${BUILD}"
else
    BUILDDIR="$(dirname "${BUILD}")"
fi

if [[ -z "${PROGRAMMER}" ]]; then
    progs=( "${BUILDDIR}"/prog_firehose_*.elf )
    [[ ${#progs[@]} -gt 0 ]] && PROGRAMMER="${progs[0]}"
fi

# Image the erase/write steps restore: an explicit QDL_HIL_IMAGE if given,
# otherwise the readback captured by the read step.
IMAGE="${IMAGE:-${SAVED_IMAGE}}"

# Optional debug logging on every qdl invocation, for post-mortems.
QDLDBG=()
[[ -n "${QDL_HIL_DEBUG}" ]] && QDLDBG=( -d )

# Common qdl prefix. -R suppresses the reset everywhere but the final step.
COMMON=( "${QDLDBG[@]}" -s "${STORAGE}" )
[[ -n "${SERIAL}" ]] && COMMON+=( -S "${SERIAL}" )

qdl_noreset() { "${QEXE}" "${COMMON[@]}" -R "$@"; }
qdl_reset()   { "${QEXE}" "${COMMON[@]}" "$@"; }

need_programmer() {
    [[ -n "${PROGRAMMER}" ]] || {
        echo "no prog_firehose_*.elf found (set QDL_HIL_PROGRAMMER)" >&2
        exit ${SKIP}
    }
}

# Print the hex sha256 of a file, using whichever tool is available.
sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "no sha256 tool (sha256sum/shasum) available" >&2
        exit ${SKIP}
    fi
}

# The nbd step's prerequisites. The hil-nbd suite checks them before
# entering EDL mode too: if the nbd step skipped, nothing would start the
# plugin whose exit resets the board, leaving it in EDL mode.
nbd_prereqs() {
    local tool
    [[ -f "${builddir}/nbdkit-qdl-plugin.so" ]] || {
        echo "nbdkit plugin not built (install nbdkit-plugin-dev)" >&2
        exit ${SKIP}
    }
    for tool in nbdkit nbdinfo nbdcopy; do
        command -v "${tool}" >/dev/null 2>&1 || {
            echo "${tool} not found (install nbdkit and libnbd-bin)" >&2
            exit ${SKIP}
        }
    done
    need_programmer
}

[[ "${step}" == nbd_list ]] && nbd_prereqs

# For unattended runs an environment-provided command puts the board into
# EDL mode ahead of the suite's first step. The list step polls for the
# device afterwards (see QDL_HIL_SETTLE), so the command only needs to
# trigger the mode switch, not wait for enumeration.
if ${FIRST_STEP} && [[ -n "${QDL_HIL_ENTER_EDL_CMD}" ]]; then
    bash -c "${QDL_HIL_ENTER_EDL_CMD}" || {
        echo "QDL_HIL_ENTER_EDL_CMD failed" >&2
        exit 1
    }
fi

# --- step implementations -------------------------------------------------

# a) Flash the whole build.
t_flash_full() {
    if ${IS_DIR}; then
        need_programmer
        local raws=( "${BUILDDIR}"/rawprogram*.xml )
        local patches=( "${BUILDDIR}"/patch*.xml )
        [[ ${#raws[@]} -gt 0 ]] || { echo "no rawprogram*.xml in ${BUILDDIR}" >&2; exit ${SKIP}; }
        qdl_noreset "${PROGRAMMER}" "${raws[@]}" "${patches[@]}"
    else
        qdl_noreset flash "${BUILD}"
    fi
}

# b) Read a partition by name (EFI by default). The readback is kept so the
# write step can restore it after erase.
t_read_partition() {
    need_programmer
    qdl_noreset "${PROGRAMMER}" read "${PARTITION}" "${SAVED_IMAGE}"
    [[ -s "${SAVED_IMAGE}" ]] || { echo "read produced an empty file" >&2; return 1; }
}

# c) Read only the GPT and confirm it really is one.
t_read_gpt() {
    need_programmer
    local out; out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN
    qdl_noreset "${PROGRAMMER}" read 0/0+34 "${out}"
    grep -qa "EFI PART" "${out}" || { echo "no GPT signature in readback" >&2; return 1; }
}

# c') Other supported read address forms.
t_read_addr_sectors() {	# P/S+L
    need_programmer
    local out; out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN
    qdl_noreset "${PROGRAMMER}" read 0/0+1 "${out}"
    [[ -s "${out}" ]] || { echo "empty readback" >&2; return 1; }
}

t_read_addr_partname() {	# P/name
    need_programmer
    local out; out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN
    qdl_noreset "${PROGRAMMER}" read "0/${PARTITION}" "${out}"
    [[ -s "${out}" ]] || { echo "empty readback" >&2; return 1; }
}

# d) Re-flash the build with --skipblock=sha256 (should mostly skip).
t_flash_skipblock() {
    if ${IS_DIR}; then
        need_programmer
        local raws=( "${BUILDDIR}"/rawprogram*.xml )
        local patches=( "${BUILDDIR}"/patch*.xml )
        [[ ${#raws[@]} -gt 0 ]] || { echo "no rawprogram*.xml" >&2; exit ${SKIP}; }
        qdl_noreset --skipblock=sha256 "${PROGRAMMER}" "${raws[@]}" "${patches[@]}"
    else
        qdl_noreset --skipblock=sha256 flash "${BUILD}"
    fi
}

# f) sha256 digest command.
t_sha256() {
    need_programmer
    local out; out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN
    qdl_noreset "${PROGRAMMER}" sha256 0/0+34 >"${out}" 2>&1
    grep -qiE '[0-9a-f]{64}' "${out}" || { echo "no sha256 digest in output:" >&2; cat "${out}" >&2; return 1; }
}

# g) Erase the partition. Only erase when there is an image to restore it with
# (the readback from the read step, or an explicit QDL_HIL_IMAGE), so the suite
# never leaves the partition wiped.
t_erase() {
    need_programmer
    [[ -f "${IMAGE}" ]] || { echo "no image to restore (run the read step or set QDL_HIL_IMAGE)" >&2; exit ${SKIP}; }
    qdl_noreset "${PROGRAMMER}" erase "${PARTITION}"
}

# f') Write the image back to its partition (the readback by default).
t_write_image() {
    need_programmer
    [[ -f "${IMAGE}" ]] || { echo "no image to write (run the read step or set QDL_HIL_IMAGE)" >&2; exit ${SKIP}; }
    qdl_noreset "${PROGRAMMER}" write "${PARTITION}" "${IMAGE}"
}

# Final step: reset the board with the explicit reset verb, ending the
# Firehose session that every other step kept alive with --skip-reset.
t_reset() {
    need_programmer
    qdl_reset "${PROGRAMMER}" reset
    rm -f "${SAVED_IMAGE}"	# done; drop the readback backup
}

# h) Enumerate EDL devices; the attached device must appear. The enter-EDL
# hook may have just power-cycled or rebooted the board, and enumeration
# after that takes several seconds - the flash steps wait on their own,
# but qdl list is a one-shot, so poll until the device shows up or
# QDL_HIL_SETTLE expires.
t_list() {
    local out deadline=$(( SECONDS + ${QDL_HIL_SETTLE:-30} ))
    out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN

    while :; do
        "${QEXE}" "${QDLDBG[@]}" list >"${out}" 2>&1 || { echo "qdl list failed" >&2; cat "${out}" >&2; return 1; }
        if ! grep -qi "No devices found" "${out}" &&
           grep -qiE '^[0-9a-f]{4}:[0-9a-f]{4}' "${out}" &&
           { [[ -z "${SERIAL}" ]] || grep -qi "${SERIAL}" "${out}"; }; then
            return 0
        fi
        if (( SECONDS >= deadline )); then
            echo "no EDL device${SERIAL:+ with serial ${SERIAL}} appeared within ${QDL_HIL_SETTLE:-30}s:" >&2
            cat "${out}" >&2
            return 1
        fi
        sleep 1
    done
}

# i) Package a contents.xml into a zip with create-zip, then flash the zip
# (the programmer comes from inside the zip).
t_create_zip() {
    [[ -n "${QDL_HIL_CONTENTS}" ]] || {
        echo "set QDL_HIL_CONTENTS to test create-zip" >&2
        exit ${SKIP}
    }
    local zip="${HIL_TMPDIR}/hil-created.zip"
    trap 'rm -f "${zip}"' RETURN
    "${QEXE}" create-zip "${zip}" "${QDL_HIL_CONTENTS}" ||
        { echo "create-zip failed" >&2; return 1; }
    [[ -s "${zip}" ]] || { echo "create-zip produced no output" >&2; return 1; }
    qdl_noreset flash "${zip}"
}

# j) Flash a build that uses sparse (Android sparse format) images.
t_sparse_flash() {
    [[ -n "${QDL_HIL_SPARSE}" && -f "${QDL_HIL_SPARSE}" ]] || {
        echo "set QDL_HIL_SPARSE to a flashmap/contents/zip using sparse images" >&2
        exit ${SKIP}
    }
    qdl_noreset flash "${QDL_HIL_SPARSE}"
}

# k) Error handling: a read of a nonexistent physical partition must be NAKed
# by the device and reported as a failure, not silently succeed.
t_nak_read() {
    need_programmer
    local out; out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN
    if qdl_noreset "${PROGRAMMER}" read 99/0+1 "${out}" >/dev/null 2>&1; then
        echo "reading nonexistent physical partition 99 unexpectedly succeeded" >&2
        return 1
    fi
}

# l) sha256 digest of a partition addressed by GPT name.
t_sha256_name() {
    need_programmer
    local out; out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN
    qdl_noreset "${PROGRAMMER}" sha256 "${PARTITION}" >"${out}" 2>&1
    grep -qiE '[0-9a-f]{64}' "${out}" ||
        { echo "no sha256 digest for ${PARTITION}:" >&2; cat "${out}" >&2; return 1; }
}

# m) Re-write the image with an overridden transfer chunk size.
t_write_chunked() {
    need_programmer
    [[ -f "${IMAGE}" ]] || {
        echo "no image to write (run the read step or set QDL_HIL_IMAGE)" >&2
        exit ${SKIP}
    }
    "${QEXE}" "${COMMON[@]}" -R --out-chunk-size=16384 \
        "${PROGRAMMER}" write "${PARTITION}" "${IMAGE}"
}

# n) Integrity check: the device's digest of the partition must match a local
# sha256 of the bytes we read back (and just wrote), proving the erase/write
# round-trip preserved the content.
t_verify_write() {
    need_programmer
    [[ -f "${SAVED_IMAGE}" ]] || {
        echo "no readback to verify against (the read step did not run)" >&2
        exit ${SKIP}
    }
    local out; out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN
    qdl_noreset "${PROGRAMMER}" sha256 "${PARTITION}" >"${out}" 2>&1 ||
        { echo "sha256 command failed" >&2; cat "${out}" >&2; return 1; }
    local dev_digest local_digest
    dev_digest="$(grep -oiE '[0-9a-f]{64}' "${out}" | head -1)"
    [[ -n "${dev_digest}" ]] || { echo "no device digest:" >&2; cat "${out}" >&2; return 1; }
    local_digest="$(sha256_file "${SAVED_IMAGE}")"
    [[ "${dev_digest,,}" == "${local_digest,,}" ]] || {
        echo "partition digest mismatch: device=${dev_digest} local=${local_digest}" >&2
        return 1
    }
}

# o) Chip identity through Sahara command mode. Must run before the
# programmer upload: chipinfo is only available while the device still
# speaks Sahara. The verb switches the device back to image-transfer mode
# on exit, so the following flash step still gets its Sahara HELLO.
t_chipinfo() {
    local out; out="$(mktemp -p "${HIL_TMPDIR}")"
    trap 'rm -f "${out}"' RETURN
    local args=()
    [[ -n "${SERIAL}" ]] && args+=( -S "${SERIAL}" )
    # The serial must follow the verb: qdl's verb dispatcher stops
    # scanning at the first argument without a leading dash, so the
    # value of a preceding "-S <serial>" derails it into the flash
    # parser. The chipinfo subcommand parses -S itself.
    if ! "${QEXE}" "${QDLDBG[@]}" chipinfo "${args[@]}" >"${out}" 2>&1; then
        if grep -qi "already in Firehose mode" "${out}"; then
            echo "programmer already running; chipinfo needs a fresh Sahara device" >&2
            exit ${SKIP}
        fi
        echo "chipinfo failed:" >&2
        cat "${out}" >&2
        return 1
    fi
    grep -qiE 'chip serial number|hw id' "${out}" ||
        { echo "chipinfo reported no chip identity:" >&2; cat "${out}" >&2; return 1; }
}

# p) Serve physical partition 0 through the nbdkit plugin and round-trip
# its primary GPT over NBD: the advertised block size must match the sector
# size the plugin logs, the GPT header must sit at LBA 1 for that sector
# size, and writing the GPT back must leave the area read back unchanged. A
# truncate filter limits the export to that area, so no client can stray
# further, and only the GPT extent its header describes is written.
t_nbd() {
    local plugin="${builddir}/nbdkit-qdl-plugin.so"
    nbd_prereqs

    # 34 sectors of 4096 bytes: room for the primary GPT at either sector
    # size, and a whole number of sectors of both.
    local area=$(( 34 * 4096 ))
    local args=( programmer="${PROGRAMMER}" storage="${STORAGE}" lun=0
                 truncate="${area}" )
    [[ -n "${SERIAL}" ]] && args+=( serial="${SERIAL}" )
    [[ -n "${QDL_HIL_DEBUG}" ]] && args+=( debug=1 )

    # Outside HIL_TMPDIR, which the next run clears: on failure this holds
    # the only copy of the original GPT area. Removed once verified.
    mkdir -p "${builddir}/meson-logs"
    local dir; dir="$(mktemp -d "${builddir}/meson-logs/hil-nbd.XXXXXX")"
    NBD_KEEP="${dir}"
    local log="${dir}/nbdkit.log"

    # The NBD client: read the area, find the GPT header at LBA 1 for
    # either sector size, write back only the header and partition entries
    # it describes, then read the whole area again.
    cat > "${dir}/client.sh" <<'CLIENT'
set -e
uri=$1
cd "${NBD_DIR}"
le() { od -An -v -t "u$2" -j "$1" -N "$2" area.img | tr -d ' '; }
nbdinfo --json "${uri}" > info.json
nbdcopy "${uri}" area.img
ss=
for s in 512 4096; do
    [[ "$(tail -c +$(( s + 1 )) area.img | head -c 8 | tr -d '\0')" == "EFI PART" ]] &&
        { ss=${s}; break; }
done
[[ -n "${ss}" ]] || { echo "no GPT header at LBA 1" >&2; exit 1; }
lba=$(le $(( ss + 72 )) 8)
count=$(le $(( ss + 80 )) 4)
size=$(le $(( ss + 84 )) 4)
gpt=$(( (lba + (count * size + ss - 1) / ss) * ss ))
(( lba >= 2 && gpt <= $(wc -c < area.img) )) || {
    echo "GPT entries (LBA ${lba}, ${count} x ${size}) outside the area" >&2
    exit 1
}
echo "writing back ${gpt} bytes of GPT"
head -c "${gpt}" area.img > gpt.img
nbdcopy gpt.img "${uri}"
nbdcopy "${uri}" readback.img
CLIENT

    # nbdkit serves until the --run command exits, then unloads the plugin,
    # which resets the board. ${uri} is set by nbdkit for that command only,
    # not exported, so hand it to the client.
    export NBD_DIR="${dir}"
    # shellcheck disable=SC2016
    if ! nbdkit -v -U - --filter=truncate "${plugin}" "${args[@]}" \
            --run 'bash "$NBD_DIR/client.sh" "$uri"' >"${log}" 2>&1; then
        echo "nbdkit session failed:" >&2
        cat "${log}" >&2
        return 1
    fi

    local geometry sector_size num_sectors block_size magic
    local re='.*serving LUN 0; sector size \([0-9]*\) bytes, \([0-9]*\) sectors.*'
    geometry="$(sed -n "s/${re}/\\1 \\2/p" "${log}" | head -1)"
    read -r sector_size num_sectors <<< "${geometry}"
    [[ -n "${sector_size}" ]] || {
        echo "plugin did not log the LUN geometry:" >&2
        cat "${log}" >&2
        return 1
    }
    echo "LUN 0: ${num_sectors} sectors of ${sector_size} bytes"
    (( num_sectors * sector_size > area )) || {
        echo "LUN 0 is smaller than the GPT area" >&2
        return 1
    }

    # Only reported when the plugin is built against nbdkit 1.30 or later
    block_size="$(sed -n 's/.*"block_size_minimum": *\([0-9]*\).*/\1/p' "${dir}/info.json")"
    if [[ -z "${block_size}" ]]; then
        echo "export advertises no block size; not checking it"
    elif [[ "${block_size}" != "${sector_size}" ]]; then
        echo "export block size ${block_size} != sector size ${sector_size}" >&2
        return 1
    fi

    magic="$(tail -c +$(( sector_size + 1 )) "${dir}/area.img" | head -c 8)"
    [[ "${magic}" == "EFI PART" ]] || {
        echo "no GPT header at LBA 1 for ${sector_size}-byte sectors" >&2
        return 1
    }
    cmp "${dir}/area.img" "${dir}/readback.img" || {
        echo "GPT area changed after writing it back over NBD" >&2
        return 1
    }

    rm -rf "${dir}"
    NBD_KEEP=""
}

# --- dispatch -------------------------------------------------------------
case "${step}" in
    list|nbd_list)      t_list ;;
    chipinfo)           t_chipinfo ;;
    flash_full)         t_flash_full ;;
    create_zip)         t_create_zip ;;
    sparse_flash)       t_sparse_flash ;;
    read_partition)     t_read_partition ;;
    read_gpt)           t_read_gpt ;;
    read_addr_sectors)  t_read_addr_sectors ;;
    read_addr_partname) t_read_addr_partname ;;
    nak_read)           t_nak_read ;;
    flash_skipblock)    t_flash_skipblock ;;
    sha256)             t_sha256 ;;
    sha256_name)        t_sha256_name ;;
    erase)              t_erase ;;
    write_image)        t_write_image ;;
    write_chunked)      t_write_chunked ;;
    verify_write)       t_verify_write ;;
    nbd)                t_nbd ;;
    reset)              t_reset ;;
    *) echo "unknown step: ${step}" >&2; exit 99 ;;
esac
