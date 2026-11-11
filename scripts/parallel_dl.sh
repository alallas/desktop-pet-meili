#!/usr/bin/env bash
# 分段并行下载（带断点续传，可反复执行直到完成）
# 用法: scripts/parallel_dl.sh <URL> <输出路径> [分段数]
# 每轮限时 265s 自动收尾（已下分段落盘），重复执行同一命令即续传
set -u
URL="$1"; OUT="$2"; PARTS="${3:-16}"
TOTAL=$(curl -sI -L -m 15 "$URL" | grep -i '^content-length' | tail -1 | tr -dc '0-9')
if [ -z "$TOTAL" ]; then
  TOTAL=$(curl -s -L -m 15 -r 0-0 -D - -o /dev/null "$URL" | grep -i '^content-range' | tail -1 | sed 's/.*\///' | tr -dc '0-9')
fi
if [ -z "$TOTAL" ]; then echo "无法获取文件大小"; exit 1; fi
CHUNK=$(( (TOTAL + PARTS - 1) / PARTS ))
PDIR="$OUT.parts"; mkdir -p "$PDIR"
echo "目标: $TOTAL 字节, $PARTS 段, 每段 $CHUNK"

fetch_part() {
  local i=$1
  local start=$(( i * CHUNK ))
  local end=$(( (i + 1) * CHUNK - 1 ))
  [ $end -ge $TOTAL ] && end=$(( TOTAL - 1 ))
  local want=$(( end - start + 1 ))
  local cur=0
  [ -f "$PDIR/part_$i" ] && cur=$(stat -c%s "$PDIR/part_$i")
  if [ "$cur" -lt "$want" ]; then
    curl -sL -m 230 -r "$(( start + cur ))-$end" -o "$PDIR/part_$i.new" "$URL"
    [ -f "$PDIR/part_$i.new" ] && cat "$PDIR/part_$i.new" >> "$PDIR/part_$i" && rm -f "$PDIR/part_$i.new"
  fi
}

for i in $(seq 0 $(( PARTS - 1 ))); do fetch_part "$i" & done
wait

DONE=1
for i in $(seq 0 $(( PARTS - 1 ))); do
  start=$(( i * CHUNK )); end=$(( (i + 1) * CHUNK - 1 )); [ $end -ge $TOTAL ] && end=$(( TOTAL - 1 ))
  want=$(( end - start + 1 ))
  cur=0; [ -f "$PDIR/part_$i" ] && cur=$(stat -c%s "$PDIR/part_$i")
  [ "$cur" -ne "$want" ] && { echo "段 $i: $cur/$want 未完成"; DONE=0; }
done

if [ "$DONE" = "1" ]; then
  cat $(for i in $(seq 0 $(( PARTS - 1 ))); do echo "$PDIR/part_$i"; done) > "$OUT"
  GOT=$(stat -c%s "$OUT")
  if [ "$GOT" = "$TOTAL" ]; then echo "✅ 完成: $OUT ($GOT 字节)"; rm -rf "$PDIR"; else echo "❌ 合并后大小不符: $GOT/$TOTAL"; exit 1; fi
else
  echo "⏳ 未完成，再执行一次本命令续传"
fi
