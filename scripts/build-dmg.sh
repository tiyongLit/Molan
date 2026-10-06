#!/usr/bin/env bash
#
# 构建 macOS DMG 安装包与自更新更新包（M 芯片 / Intel / 两者）。
#
# 用法：
#   pnpm build:mac                          构建两个架构，产物收集到 release/
#   pnpm build:mac:arm                      仅构建 M 芯片 (aarch64)
#   pnpm build:mac:intel                    仅构建 Intel (x86_64)
#   bash scripts/build-dmg.sh --dry-run     仅同步版本号并打印构建计划，不编译
#
# 版本号单一事实源：package.json 的 version。
# 本脚本会把它同步写入 src-tauri/tauri.conf.json 与 src-tauri/Cargo.toml，
# 保证 dmg 文件名、app 版本、自更新 current_version 三处一致。
#
# 自更新更新包：构建必须能签名（tauri.conf.json 已启用 createUpdaterArtifacts），
# 私钥默认读 ~/.molan-key.txt，可用 MOLAN_SIGNING_KEY_FILE 或
# TAURI_SIGNING_PRIVATE_KEY（私钥内容/路径）环境变量覆盖。更新包收集时重命名为
# Molan_<version>_<arch>.app.tar.gz 并附带同名 .sig，最后由
# scripts/gen-latest-json.sh 直接写入发布清单 update/latest.json。
#
# 产物收集语义：release/ 下同名文件直接覆盖——不检测历史包、不重命名，
# 版本号没改时重跑即覆盖旧包（更新包因带版本+架构后缀，天然区分）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT_DIR="release"

# ── 参数解析 ─────────────────────────────────────────────────────
ARCH=""
DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --arch=*) ARCH="${arg#--arch=}" ;;
    --dry-run) DRY_RUN=1 ;;
    *)
      echo "未知参数：${arg}（可用：--arch=arm|intel，--dry-run）" >&2
      exit 1
      ;;
  esac
done
case "$ARCH" in
  "" | arm | intel) ;;
  *)
    echo "未知架构 \"$ARCH\"，可选值：arm | intel" >&2
    exit 1
    ;;
esac

triples=()
chips=()
arch_tags=()
if [ -z "$ARCH" ] || [ "$ARCH" = "arm" ]; then
  triples+=("aarch64-apple-darwin")
  chips+=("M 芯片 (Apple Silicon)")
  arch_tags+=("aarch64")
fi
if [ -z "$ARCH" ] || [ "$ARCH" = "intel" ]; then
  triples+=("x86_64-apple-darwin")
  chips+=("Intel")
  arch_tags+=("x64")
fi

# ── 1. 读取版本号（单一事实源：package.json） ─────────────────────
VERSION="$(grep -m1 '"version"' package.json | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
if [ -z "$VERSION" ]; then
  echo "package.json 的 version 缺失，无法构建" >&2
  exit 1
fi
echo "[build-dmg] 当前版本：$VERSION"

# ── 2. 同步版本号到 tauri.conf.json / Cargo.toml ─────────────────
# 仅当内容变化时才写入，避免无谓的 mtime 变更触发增量重编译。
CONF="src-tauri/tauri.conf.json"
CONF_VERSION="$(grep -m1 '"version"' "$CONF" | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
if [ "$CONF_VERSION" != "$VERSION" ]; then
  VERSION="$VERSION" perl -i -pe 'if (!$done) { s/("version":\s*")[^"]+(")/$1$ENV{VERSION}$2/ and $done = 1 }' "$CONF"
  echo "[build-dmg] 已同步 tauri.conf.json → $VERSION"
fi

CARGO="src-tauri/Cargo.toml"
CARGO_VERSION="$(grep -m1 -E '^version[[:space:]]*=' "$CARGO" | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
if [ "$CARGO_VERSION" != "$VERSION" ]; then
  VERSION="$VERSION" perl -i -pe 'if (!$done) { s/^(version = ")[^"]+(")/$1$ENV{VERSION}$2/ and $done = 1 }' "$CARGO"
  echo "[build-dmg] 已同步 Cargo.toml → $VERSION"
fi

# ── 3. 签名私钥注入（自更新更新包签名） ───────────────────────────
# createUpdaterArtifacts 启用后，构建必须能签名，否则 tauri build 直接失败。
# 坑 1：tauri build 只认 TAURI_SIGNING_PRIVATE_KEY（值为私钥内容或路径），
#       不读 TAURI_SIGNING_PRIVATE_KEY_PATH（那是 signer 子命令专用）。
# 坑 2：两个变量同时存在会让 signer 报 "cannot be used with"（exit 2），
#       故统一只注入 TAURI_SIGNING_PRIVATE_KEY（内容），外部误设的 _PATH 会被转注后清除。
# 解析顺序：已有 TAURI_SIGNING_PRIVATE_KEY → ~/.molan-key.txt（可用 MOLAN_SIGNING_KEY_FILE 覆盖）。
KEY_FILE="${MOLAN_SIGNING_KEY_FILE:-$HOME/.molan-key.txt}"
KEY_FROM_ENV=0
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  KEY_FROM_ENV=1
fi
if [ -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ]; then
  if [ "$KEY_FROM_ENV" -eq 0 ] && [ -f "${TAURI_SIGNING_PRIVATE_KEY_PATH}" ]; then
    export TAURI_SIGNING_PRIVATE_KEY="$(cat "${TAURI_SIGNING_PRIVATE_KEY_PATH}")"
    echo "[build-dmg] 签名私钥：来自 TAURI_SIGNING_PRIVATE_KEY_PATH"
  fi
  unset TAURI_SIGNING_PRIVATE_KEY_PATH
fi
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  if [ -f "$KEY_FILE" ]; then
    export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"
    echo "[build-dmg] 签名私钥：$KEY_FILE"
  elif [ "$DRY_RUN" -eq 1 ]; then
    echo "[build-dmg] 警告：未找到签名私钥 ${KEY_FILE}（dry-run 继续；真实构建会失败）"
  else
    echo "[build-dmg] 错误：未找到签名私钥（$KEY_FILE 不存在）" >&2
    echo "  请先运行：pnpm tauri signer generate -w ~/.molan-key.txt" >&2
    echo "  或设置环境变量 TAURI_SIGNING_PRIVATE_KEY（私钥内容或路径）" >&2
    exit 1
  fi
elif [ "$KEY_FROM_ENV" -eq 1 ]; then
  echo "[build-dmg] 签名私钥：使用已有的 TAURI_SIGNING_PRIVATE_KEY 环境变量"
fi
# 签名密码解析：环境变量 → ${KEY_FILE}.password 文件（须非空）→ 空（无密码私钥）。
# 私钥有密码时必须命中前两者之一，否则下方签名预检会提前失败。
PW_FILE="${MOLAN_SIGNING_KEY_PASSWORD_FILE:-${KEY_FILE}.password}"
PW_SOURCE=""
if [ -n "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]; then
  PW_SOURCE="环境变量"
elif [ -s "$PW_FILE" ] && [ -n "$(cat "$PW_FILE")" ]; then
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat "$PW_FILE")"
  PW_SOURCE="$PW_FILE"
fi
if [ -n "$PW_SOURCE" ]; then
  echo "[build-dmg] 签名密码：已就绪（来源：${PW_SOURCE}）"
else
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
  echo "[build-dmg] 签名密码：无（私钥未设密码，属正常）"
fi

# 签名预检：几秒内验证私钥+密码可用，避免长构建跑完才在打包签名阶段失败。
if [ "$DRY_RUN" -eq 0 ]; then
  CHECK_FILE="$(mktemp -t molan-signcheck)"
  printf 'signcheck' > "$CHECK_FILE"
  if CI=true pnpm tauri signer sign "$CHECK_FILE" >/dev/null 2>&1; then
    echo "[build-dmg] 签名预检通过"
    rm -f "$CHECK_FILE" "$CHECK_FILE.sig"
  else
    rm -f "$CHECK_FILE" "$CHECK_FILE.sig"
    echo "[build-dmg] 错误：签名预检失败——私钥与密码不匹配。" >&2
    echo "  检查 TAURI_SIGNING_PRIVATE_KEY_PASSWORD / ${PW_FILE}；若私钥已无密码，请删除该密码文件" >&2
    exit 1
  fi
fi

# ── 4. 依次构建各架构 ─────────────────────────────────────────────
# CI=true 使 tauri-bundler 对 dmg 打包传 --skip-jenkins：跳过 Finder 挂载 +
# AppleScript 布局，不再弹出"拖拽安装"窗口（dmg 内容不变，仅无自定义背景与
# 图标预排位置）。如需恢复完整布局（会弹窗），运行前设置 TAURI_BUNDLER_DMG_IGNORE_CI=1。
i=0
while [ "$i" -lt "${#triples[@]}" ]; do
  echo "[build-dmg] 开始构建 ${chips[$i]}（${triples[$i]}）…"
  if [ "$DRY_RUN" -eq 0 ]; then
    CI=true pnpm tauri build --target "${triples[$i]}"
    # 签名校验：产物必须携带有效代码签名（ad-hoc 或 Developer ID）。
    # 未签名 app 的 TCC 身份校验失败（tccd 报 -67062），完全磁盘访问等授权
    # 全部无法生效（曾致生产包废纸篓读取 EPERM），必须阻断出包。
    APP_BUNDLE="target/${triples[$i]}/release/bundle/macos/Molan.app"
    if ! codesign --verify "$APP_BUNDLE" >/dev/null 2>&1; then
      echo "[build-dmg] 错误：${chips[$i]} 产物缺少有效代码签名" >&2
      echo "  检查 tauri.conf.json 的 bundle.macOS.signingIdentity 是否为 \"-\"（ad-hoc）" >&2
      exit 1
    fi
    echo "[build-dmg] ${chips[$i]} 签名校验通过：$(codesign -dv "$APP_BUNDLE" 2>&1 | grep -E 'Identifier=|Signature=' | tr '\n' ' ')"
  fi
  i=$((i + 1))
done

# ── 5. 收集产物到 release/（dmg + 自更新更新包，同名覆盖） ────────
if [ "$DRY_RUN" -eq 1 ]; then
  echo "[build-dmg] --dry-run：跳过构建与产物收集"
  exit 0
fi

mkdir -p "$OUT_DIR"
echo ""
echo "[build-dmg] 构建完成，产物如下："
i=0
while [ "$i" -lt "${#triples[@]}" ]; do
  triple="${triples[$i]}"
  chip="${chips[$i]}"
  arch_tag="${arch_tags[$i]}"
  dmg_dir="target/$triple/release/bundle/dmg"
  found=0
  # 只收集当前版本的 dmg，防目录中旧版本残留被误收；glob 无匹配时保持字面量，需跳过
  for dmg in "$dmg_dir"/*_"$VERSION"_*.dmg; do
    [ -e "$dmg" ] || continue
    name="$(basename "$dmg")"
    cp -f "$dmg" "$OUT_DIR/$name" # 同名直接覆盖
    mb="$(awk -v b="$(stat -f%z "$OUT_DIR/$name")" 'BEGIN { printf "%.1f", b / 1048576 }')"
    printf '  %-24s %s/%s  (%s MB)\n' "$chip" "$OUT_DIR" "$name" "$mb"
    found=1
  done
  if [ "$found" -eq 0 ]; then
    echo "[build-dmg] 错误：$chip 构建产物缺失，未在 $dmg_dir 找到版本 $VERSION 的 dmg" >&2
    exit 1
  fi

  # 自更新更新包（.app.tar.gz + .sig）：重命名带 版本+架构 后缀。
  # 双架构产物同名（Molan.app.tar.gz），不加后缀上传同一 Release 会冲突。
  mac_dir="target/$triple/release/bundle/macos"
  tfound=0
  for tarball in "$mac_dir"/*.app.tar.gz; do
    [ -e "$tarball" ] || continue
    if [ ! -f "$tarball.sig" ]; then
      echo "[build-dmg] 错误：缺少 $tarball.sig——签名未生效，检查私钥设置" >&2
      exit 1
    fi
    base="$(basename "$tarball" .app.tar.gz)"
    upname="${base}_${VERSION}_${arch_tag}.app.tar.gz"
    cp -f "$tarball" "$OUT_DIR/$upname"
    cp -f "$tarball.sig" "$OUT_DIR/$upname.sig"
    mb="$(awk -v b="$(stat -f%z "$OUT_DIR/$upname")" 'BEGIN { printf "%.1f", b / 1048576 }')"
    printf '  %-24s %s/%s  (%s MB) + .sig\n' "$chip" "$OUT_DIR" "$upname" "$mb"
    tfound=1
  done
  if [ "$tfound" -eq 0 ]; then
    echo "[build-dmg] 错误：$chip 未生成 .app.tar.gz——确认 tauri.conf.json 的" >&2
    echo "  bundle.createUpdaterArtifacts 为 true 且签名私钥有效" >&2
    exit 1
  fi
  i=$((i + 1))
done

# ── 6. 生成自更新发布清单（update/latest.json） ───────────────────
bash scripts/gen-latest-json.sh "$VERSION"
