#!/usr/bin/env bash
set -euo pipefail

RADAR_OWNER="2mlml"
RADAR_REPO="cs2-radar-images"
RADAR_BRANCH="master"
RADAR_API_DIR_URL="https://api.github.com/repos/${RADAR_OWNER}/${RADAR_REPO}/contents"
RADAR_API_COMMIT_URL="https://api.github.com/repos/${RADAR_OWNER}/${RADAR_REPO}/commits/${RADAR_BRANCH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RADAR_DEST_DIR="${ROOT_DIR}/assets/radar/radars"
OVERVIEW_DEST_DIR="${ROOT_DIR}/assets/radar/overviews"
JSON_DEST_DIR="${ROOT_DIR}/assets/radar/json"
RADAR_MANIFEST_PATH="${RADAR_DEST_DIR}/manifest.json"
OVERVIEW_MANIFEST_PATH="${OVERVIEW_DEST_DIR}/manifest.json"
JSON_MANIFEST_PATH="${JSON_DEST_DIR}/manifest.json"
TMP_DIR="$(mktemp -d)"
USER_AGENT="cs2-radar-updater"

cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

mkdir -p "${RADAR_DEST_DIR}" "${OVERVIEW_DEST_DIR}" "${JSON_DEST_DIR}"

curl_json() {
  curl -fsSL -H "User-Agent: ${USER_AGENT}" "$1"
}

write_manifest() {
  local manifest_path="$1"
  local source_repo="$2"
  local source_branch="$3"
  local source_directory="$4"
  local source_commit="$5"
  local urls_file="$6"
  local dest_dir="$7"

  {
    printf '{\n'
    printf '  "source_repo": "%s",\n' "${source_repo}"
    printf '  "source_branch": "%s",\n' "${source_branch}"
    printf '  "source_directory": "%s",\n' "${source_directory}"
    printf '  "source_commit": "%s",\n' "${source_commit}"
    printf '  "files": [\n'

    local first=1
    while IFS= read -r url; do
      local file_name size_bytes
      file_name="$(basename "${url}")"
      size_bytes="$(wc -c < "${dest_dir}/${file_name}" | tr -d '[:space:]')"
      if [[ "${first}" -eq 0 ]]; then
        printf ',\n'
      fi
      first=0
      printf '    {\n'
      printf '      "name": "%s",\n' "${file_name}"
      printf '      "download_url": "%s",\n' "${url}"
      printf '      "size": %s\n' "${size_bytes}"
      printf '    }'
    done < "${urls_file}"

    printf '\n  ]\n'
    printf '}\n'
  } > "${manifest_path}"
}

write_generated_manifest() {
  local manifest_path="$1"
  local source_repo="$2"
  local source_branch="$3"
  local source_directory="$4"
  local source_commit="$5"
  local urls_file="$6"
  local dest_dir="$7"

  {
    printf '{\n'
    printf '  "source_repo": "%s",\n' "${source_repo}"
    printf '  "source_branch": "%s",\n' "${source_branch}"
    printf '  "source_directory": "%s",\n' "${source_directory}"
    printf '  "source_commit": "%s",\n' "${source_commit}"
    printf '  "generated_from": "overview txt",\n'
    printf '  "files": [\n'

    local first=1
    while IFS= read -r url; do
      local txt_name json_name size_bytes
      txt_name="$(basename "${url}")"
      json_name="${txt_name%.txt}.json"
      size_bytes="$(wc -c < "${dest_dir}/${json_name}" | tr -d '[:space:]')"
      if [[ "${first}" -eq 0 ]]; then
        printf ',\n'
      fi
      first=0
      printf '    {\n'
      printf '      "name": "%s",\n' "${json_name}"
      printf '      "source_overview_url": "%s",\n' "${url}"
      printf '      "size": %s\n' "${size_bytes}"
      printf '    }'
    done < "${urls_file}"

    printf '\n  ]\n'
    printf '}\n'
  } > "${manifest_path}"
}

sync_files() {
  local url_pattern="$1"
  local listing_path="$2"
  local urls_path="$3"
  local dest_dir="$4"
  local extension="$5"

  grep -o "${url_pattern}" "${listing_path}" | sort -u > "${urls_path}"

  if [[ ! -s "${urls_path}" ]]; then
    echo "no upstream ${extension} files found" >&2
    exit 1
  fi

  while IFS= read -r existing; do
    [[ -e "${existing}" ]] || continue
    local base
    base="$(basename "${existing}")"
    if ! grep -qx "https://raw.githubusercontent.com/.*/${base}" "${urls_path}"; then
      rm -f "${existing}"
    fi
  done < <(find "${dest_dir}" -maxdepth 1 -type f -name "*.${extension}" | sort)

  while IFS= read -r url; do
    local file_name
    file_name="$(basename "${url}")"
    curl -fsSL -H "User-Agent: ${USER_AGENT}" "${url}" -o "${dest_dir}/${file_name}"
    echo "downloaded ${file_name}"
  done < "${urls_path}"
}

generate_json_transforms() {
  local source_dir="$1"
  local dest_dir="$2"

  while IFS= read -r existing; do
    [[ -e "${existing}" ]] || continue
    local base
    base="$(basename "${existing}")"
    if [[ "${base}" == "manifest.json" ]]; then
      continue
    fi
    local stem="${base%.json}"
    if [[ ! -f "${source_dir}/${stem}.txt" ]]; then
      rm -f "${existing}"
    fi
  done < <(find "${dest_dir}" -maxdepth 1 -type f -name "*.json" | sort)

  while IFS= read -r overview_path; do
    [[ -e "${overview_path}" ]] || continue
    local map_key json_path
    map_key="$(basename "${overview_path}" .txt)"
    json_path="${dest_dir}/${map_key}.json"

    awk '
      BEGIN {
        pos_x = "0.0";
        pos_y = "0.0";
        scale = "1.0";
        rotate = "0";
        zoom = "0.0";
      }
      {
        field_count = split($0, parts, "\"");
        if (field_count >= 4) {
          key = parts[2];
          val = parts[4];
          if (key == "pos_x") pos_x = val;
          else if (key == "pos_y") pos_y = val;
          else if (key == "scale") scale = val;
          else if (key == "rotate") rotate = val;
          else if (key == "zoom") zoom = val;
        }
      }
      END {
        printf "{\n";
        printf "  \"pos_x\": %s,\n", pos_x;
        printf "  \"pos_y\": %s,\n", pos_y;
        printf "  \"scale\": %s,\n", scale;
        printf "  \"rotate\": %s,\n", rotate;
        printf "  \"zoom\": %s\n", zoom;
        printf "}\n";
      }
    ' "${overview_path}" > "${json_path}"

    echo "generated $(basename "${json_path}")"
  done < <(find "${source_dir}" -maxdepth 1 -type f -name "*.txt" | sort)
}

radar_commit_sha="$(
  curl_json "${RADAR_API_COMMIT_URL}" |
    sed -n 's/.*"sha"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
    head -n 1
)"

[[ -n "${radar_commit_sha}" ]] || { echo "failed to resolve radar source commit sha" >&2; exit 1; }

radar_listing_path="${TMP_DIR}/radar_listing.json"

curl_json "${RADAR_API_DIR_URL}" > "${radar_listing_path}"

radar_urls="${TMP_DIR}/radar_urls.txt"
overview_urls="${TMP_DIR}/overview_urls.txt"

sync_files 'https://raw.githubusercontent.com/[^"]*\.png' "${radar_listing_path}" "${radar_urls}" "${RADAR_DEST_DIR}" "png"
sync_files 'https://raw.githubusercontent.com/[^"]*\.txt' "${radar_listing_path}" "${overview_urls}" "${OVERVIEW_DEST_DIR}" "txt"
generate_json_transforms "${OVERVIEW_DEST_DIR}" "${JSON_DEST_DIR}"

write_manifest "${RADAR_MANIFEST_PATH}" \
  "${RADAR_OWNER}/${RADAR_REPO}" \
  "${RADAR_BRANCH}" \
  "." \
  "${radar_commit_sha}" \
  "${radar_urls}" \
  "${RADAR_DEST_DIR}"

write_manifest "${OVERVIEW_MANIFEST_PATH}" \
  "${RADAR_OWNER}/${RADAR_REPO}" \
  "${RADAR_BRANCH}" \
  "." \
  "${radar_commit_sha}" \
  "${overview_urls}" \
  "${OVERVIEW_DEST_DIR}"

write_generated_manifest "${JSON_MANIFEST_PATH}" \
  "${RADAR_OWNER}/${RADAR_REPO}" \
  "${RADAR_BRANCH}" \
  "generated-json" \
  "${radar_commit_sha}" \
  "${overview_urls}" \
  "${JSON_DEST_DIR}"

radar_count="$(wc -l < "${radar_urls}" | tr -d '[:space:]')"
overview_count="$(wc -l < "${overview_urls}" | tr -d '[:space:]')"
json_count="$(find "${JSON_DEST_DIR}" -maxdepth 1 -type f -name '*.json' ! -name 'manifest.json' | wc -l | tr -d '[:space:]')"
echo "updated ${radar_count} radar images into ${RADAR_DEST_DIR}"
echo "updated ${overview_count} overview txt files into ${OVERVIEW_DEST_DIR}"
echo "generated ${json_count} radar json files into ${JSON_DEST_DIR}"
