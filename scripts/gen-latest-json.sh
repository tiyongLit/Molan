#!/usr/bin/env bash
#
# 生成自更新清单 release/latest.json（tauri-plugin-updater 静态格式）。
#
# 输入：release/ 下由 scripts/build-dmg.sh 收集的
#   Molan_<version>_<arch>.app.tar.gz（+ 同名 .sig），arch ∈ {aarch64, x64}
# 输出：release/latest.json（signature 直接取 .sig 文件内容，单行 base64）
#
# 用法：
#   bash scripts/gen-latest-json.sh                # 版本号取 package.json
#   bash scripts/gen-latest-json.sh 1.0.0-alpha.2  # 显式指定版本
#
# 环境变量：
#   GITEE_REPO   仓库地址，默认 https://gitee.com/tiyong/Molan
#   RELEASE_TAG  Release tag，默认与版本号相同
#
# 发布顺序（重要）：先建 Gitee Release 并上传更新包与 dmg，再把 latest.json
# 拷贝到 update/latest.json 提交推送——顺序反了用户会下载到 404。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT_DIR="release"

GITEE_REPO="${GITEE_REPO:-https://gitee.com/tiyong/Molan}"
GITEE_REPO="${GITEE_REPO%/}"
VERSION="${1:-$(grep -m1 '"version"' package.json | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')}"
RELEASE_TAG="${RELEASE_TAG:-$VERSION}"
if [ -z "$VERSION" ] || [ -z "$RELEASE_TAG" ]; then
  echo "[gen-latest] 错误：无法确定版本号（package.json 无 version 且未传参数）" >&2
  exit 1
fi

# ── 收集更新包条目（文件名里的架构标记 → 平台键） ─────────────────
# x64 → darwin-x86_64，aarch64 → darwin-aarch64（与 tauri 平台键一致）
entries=()
for tarball in "$OUT_DIR"/Molan_"$VERSION"_*.app.tar.gz; do
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

  url="$GITEE_REPO/releases/download/$RELEASE_TAG/$name"
  entries+=("$(printf '    "%s": {\n      "signature": "%s",\n      "url": "%s"\n    }' "$platform" "$signature" "$url")")
done

if [ "${#entries[@]}" -eq 0 ]; then
  echo "[gen-latest] 错误：release/ 下未找到版本 $VERSION 的 .app.tar.gz" >&2
  echo "  请先运行 pnpm build:mac（构建会自动收集并重命名更新包）" >&2
  exit 1
fi

# ── 组装 JSON ─────────────────────────────────────────────────────
pub_date="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
{
  printf '{\n'
  printf '  "version": "%s",\n' "$VERSION"
  printf '  "pub_date": "%s",\n' "$pub_date"
  printf '  "platforms": {\n'
  i=0
  while [ "$i" -lt "${#entries[@]}" ]; do
    printf '%s' "${entries[$i]}"
    i=$((i + 1))
    if [ "$i" -lt "${#entries[@]}" ]; then
      printf ',\n'
    else
      printf '\n'
    fi
  done
  printf '  }\n}\n'
} > "$OUT_DIR/latest.json"

if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$OUT_DIR/latest.json" > /dev/null
  echo "[gen-latest] JSON 校验通过"
fi

echo "[gen-latest] 已生成 $OUT_DIR/latest.json（版本 ${VERSION}，tag ${RELEASE_TAG}，平台数 ${#entries[@]}）"
if [ "${#entries[@]}" -lt 2 ]; then
  echo "[gen-latest] 注意：仅包含 1 个平台键——双架构发布请两个架构都构建后再跑一次" >&2
fi
echo "[gen-latest] 发布下一步："
echo "  1. Gitee 建 Release（tag: ${RELEASE_TAG}），上传 release/ 下 *.app.tar.gz 与 dmg"
echo "  2. cp $OUT_DIR/latest.json update/latest.json && git add update/latest.json"
echo "     git commit -m \"chore(release): $RELEASE_TAG\" && git push"
echo "  3. 约 1 分钟后验证：curl -sL $GITEE_REPO/raw/master/update/latest.json"
