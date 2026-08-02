#!/usr/bin/env bash
# 验证 STM32 类型系统在编译期拦截错误引脚（需要 thumbv7em-none-eabihf target）。
#
# 原理：临时把芯片级 dtsi 里 I2C1 的 SCL 改成非法引脚 PA0，
# 期望 `cargo check` 失败且报出 `SclPin` trait 错误，随后恢复原文件。

set -euo pipefail

cd "$(dirname "$0")/../stm32"

DTS="boards/stm32h723.dtsi"
cp "$DTS" "$DTS.compile-fail.bak"
trap 'mv -f "$DTS.compile-fail.bak" "$DTS"' EXIT

perl -pi -e 's/scl = "PB8"/scl = "PA0"/' "$DTS"

LOG="$(mktemp -t dt-compile-fail.XXXXXX)"
trap 'mv -f "$DTS.compile-fail.bak" "$DTS"; rm -f "$LOG"' EXIT

cargo check --offline --target thumbv7em-none-eabihf --example h723_nucleo >"$LOG" 2>&1 || true

if grep -q "SclPin" "$LOG"; then
    echo "PASS: wrong pin was rejected at compile time"
else
    echo "FAIL: expected a SclPin compile error" >&2
    cat "$LOG" >&2
    exit 1
fi
