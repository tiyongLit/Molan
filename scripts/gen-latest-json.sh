#!/usr/bin/env bash
#
# 生成自更新发布清单 update/latest.json（tauri-plugin-updater 静态格式）。
#
# 输入：release/ 下由 scripts/build-dmg.sh 收集的
#   Molan_<version>_<arch>.app.tar.gz（+ 同名 .sig），arch ∈ {aarch64, x64}
# 输出：update/latest.json —— 唯一清单文件（随 Git 提交推送后由 App 读取），
#   signature 直接取 .sig 文件内容（单行 base64）；release/ 不再产出任何 json。
#
# 用法：
#   bash scripts/gen-latest-json.sh                # 版本号取 package.json
#   bash scripts/gen-latest-json.sh 1.0.0-alpha.2  # 显式指定版本
#
# 环境变量：
#   GITEE_REPO   主源仓库地址，默认 https://gitee.com/tiyong/Molan
#   GITHUB_REPO  备用源口子（当前留空、不启用）。填入 GitHub 仓库地址后，脚本会
#                额外生成 release/latest.github.json（url 指向 GitHub Release）；
#                将来开通 GitHub 发布时，把该文件内容放到 GitHub 仓库的
#                update/latest.json 即可（tauri.conf.json 的备源端点已就位）。
#   RELEASE_TAG  Release tag，默认与版本号相同
#
# 发布顺序（重要）：先建 Gitee Release 并上传更新包与 dmg，再提交推送
# update/latest.json——顺序反了用户会下载到 404。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SRC_DIR="release"                 # 更新包（.app.tar.gz + .sig）所在目录
MANIFEST_OUT="update/latest.json" # 唯一发布清单（提交进 Git，App 通过 raw URL 读取）

GITEE_REPO="${GITEE_REPO:-https://gitee.com/tiyong/Molan}"
GITEE_REPO="${GITEE_REPO%/}"
GITHUB_REPO="${GITHUB_REPO:-}"
GITHUB_REPO="${GITHUB_REPO%/}"
VERSION="${1:-$(grep -m1 '"version"' package.json | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')}"
RELEASE_TAG="${RELEASE_TAG:-$VERSION}"
if [ -z "$VERSION" ] || [ -z "$RELEASE_TAG" ]; then
  echo "[gen-latest] 错误：无法确定版本号（package.json 无 version 且未传参数）" >&2
  exit 1
fi

# ── 收集更新包条目（文件名里的架构标记 → 平台键） ─────────────────
# x64 → darwin-x86_64，aarch64 → darwin-aarch64（与 tauri 平台键一致）。
# 结果写入全局数组 ENTRIES（Gitee / GitHub 两套 url 复用同一收集逻辑）。
ENTRIES=()
collect_entries() {
  local repo="$1"
  local tarball name platform sig_file signature
  ENTRIES=()
  for tarball in "$SRC_DIR"/Molan_"$VERSION"_*.app.tar.gz; do
    [ -e "$tarball" ] || continue
    name="$(basename "$tarball")"
    case "$name" in
      *_aarch64.app.tar.gz) platform="darwin-aarch64" ;;
      *_x64.app.tar.gz) platform="darwin-x86_64" ;;
      *)
        echo "[gen-latest] 跳过无法识别架构的文件：$name" >&2
        continue
        ;;
    esac

    sig_file="$tarball.sig"
    if [ ! -f "$sig_file" ]; then
      echo "[gen-latest] 错误：缺少签名文件 $sig_file" >&2
      exit 1
    fi
    signature="$(tr -d '\r\n' < "$sig_file")"
    case "$signature" in
      "" | *[!A-Za-z0-9+/=]*)
        echo "[gen-latest] 错误：$sig_file 内容不是合法 base64" >&2
        exit 1
        ;;
    esac

    ENTRIES+=("$(printf '    "%s": {\n      "signature": "%s",\n      "url": "%s"\n    }' \
      "$platform" "$signature" "$repo/releases/download/$RELEASE_TAG/$name")")
  done
}

# ── 组装并写出清单 JSON（python3 可用时顺带校验） ────────────────
write_manifest() {
  local out="$1" pub_date i
  mkdir -p "$(dirname "$out")"
  pub_date="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  {
    printf '{\n'
    printf '  "version": "%s",\n' "$VERSION"
    printf '  "pub_date": "%s",\n' "$pub_date"
    printf '  "platforms": {\n'
    i=0
    while [ "$i" -lt "${#ENTRIES[@]}" ]; do
      printf '%s' "${ENTRIES[$i]}"
      i=$((i + 1))
      if [ "$i" -lt "${#ENTRIES[@]}" ]; then
        printf ',\n'
      else
        printf '\n'
      fi
    done
    printf '  }\n}\n'
  } > "$out"

  if command -v python3 >/dev/null 2>&1; then
    python3 -m json.tool "$out" > /dev/null
    echo "[gen-latest] $out JSON 校验通过"
  fi
}

# ── 主流程：Gitee 主源（必须） → GitHub 备用源草稿（可选，GITHUB_REPO 非空时） ──
collect_entries "$GITEE_REPO"
if [ "${#ENTRIES[@]}" -eq 0 ]; then
  echo "[gen-latest] 错误：release/ 下未找到版本 $VERSION 的 .app.tar.gz" >&2
  echo "  请先运行 pnpm build:mac（构建会自动收集并重命名更新包）" >&2
  exit 1
fi

write_manifest "$MANIFEST_OUT"
echo "[gen-latest] 已生成 ${MANIFEST_OUT}（版本 ${VERSION}，tag ${RELEASE_TAG}，平台数 ${#ENTRIES[@]}）"
if [ "${#ENTRIES[@]}" -lt 2 ]; then
  echo "[gen-latest] 注意：仅包含 1 个平台键——双架构发布请两个架构都构建后再跑一次" >&2
fi

if [ -n "$GITHUB_REPO" ]; then
  collect_entries "$GITHUB_REPO"
  write_manifest "release/latest.github.json"
  echo "[gen-latest] 已生成 release/latest.github.json（GitHub 备用源草稿）"
fi

echo "[gen-latest] 发布下一步："
echo "  1. Gitee 建 Release（tag: ${RELEASE_TAG}），上传 release/ 下 *.app.tar.gz 与 dmg"
echo "  2. git add update/latest.json && git commit -m \"chore(release): ${RELEASE_TAG}\" && git push"
echo "  3. 约 1 分钟后验证：curl -sL $GITEE_REPO/raw/master/update/latest.json"
