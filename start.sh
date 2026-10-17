#!/bin/bash
cd "$(dirname "$0")"

# フォロー先を集める対象アカウント（第1引数で変更可: ./start.sh 別のユーザー名）
TARGET_USER="${1:-TWC-ils}"

echo "Rustをビルド中..."
cargo build --release
if [ $? -ne 0 ]; then
  echo "ビルド失敗"
  exit 1
fi

echo ""
echo "=== 取得フロー ==="
echo "  1. ${TARGET_USER} がフォローしているユーザーを収集"
echo "  2. 重複除去してから全ユーザーの全作品を取得"
echo "  3. DBに取り込んで検索サーバーを起動"
echo ""

# データ取得（完了するまで待機）
echo "データ取得を開始..."
./target/release/scratch-mega-fetcher "$TARGET_USER"
if [ $? -ne 0 ]; then
  echo "データ取得に失敗しました"
  exit 1
fi

echo ""
echo "Bunサーバーを起動中..."
bun run server.ts &
BUN_PID=$!

echo ""
echo "起動完了！"
echo "   検索サイト → http://localhost:5001"
echo "   止めるには Ctrl+C"
echo ""

trap "echo '停止中...'; kill $BUN_PID 2>/dev/null; exit" INT TERM
wait $BUN_PID
