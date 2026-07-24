#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

yt_dlp="${SHARE_WITH_ME_YTDLP:-}"
if [[ -z "$yt_dlp" ]] && command -v yt-dlp >/dev/null 2>&1; then
  yt_dlp="$(command -v yt-dlp)"
fi

strict_urls=(
  'https://xhslink.com/o/830zFJhXqUv'
  'https://www.toutiao.com/article/7651359327906710016/'
  'https://www.bilibili.com/video/BV1epwvzGEKf/'
  'https://www.zhihu.com/question/2017210364081754127/answer/2028521919280858844'
  'https://weibo.com/2656274875/5265499613041182'
  'https://jingxuan.douyin.com/m/video/7644217367635037440'
  'https://x.com/Interior/status/463440424141459456'
  'https://www.instagram.com/p/fA9uwTtkSN/'
  'https://movie.douban.com/subject/1292052/'
  'https://ditu.amap.com/place/B000A8UIN8'
  'https://item.jd.com/10203710219600.html'
  'https://weread.qq.com/web/bookDetail/02932980813ab7a43g012e77'
)

youtube_url='https://www.youtube.com/watch?v=dQw4w9WgXcQ'

share_required_urls=(
  'http://j.map.baidu.com/BkmBk'
  'https://item.taobao.com/item.htm?id=752598787556'
  'https://detail.tmall.com/item.htm?id=821944906491'
)

cargo build --locked --quiet
if [[ -z "$yt_dlp" ]]; then
  echo 'yt-dlp is required for the live video transcript contract.' >&2
  echo 'Set SHARE_WITH_ME_YTDLP to the pinned production binary.' >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo 'jq is required to validate live probe JSON.' >&2
  exit 1
fi

SHARE_WITH_ME_YTDLP="$yt_dlp" target/debug/backend --probe-strict "${strict_urls[@]}"

youtube_result="$(
  SHARE_WITH_ME_YTDLP="$yt_dlp" target/debug/backend --probe-strict "$youtube_url"
)"
printf '%s\n' "$youtube_result"
jq -e '
  .platform == "YouTube"
  and .method == "yt-dlp"
  and (.transcript_source == "平台字幕" or .transcript_source == "语音转写")
  and .original_chars >= 100
' <<<"$youtube_result" >/dev/null

# Keep URL-only behavior visible for sources that intentionally rely on the
# richer text supplied by the system share sheet when public pages are gated.
target/debug/backend --probe "${share_required_urls[@]}"

target/debug/backend --probe-shared-strict \
  'http://j.map.baidu.com/BkmBk' \
  '百度地图分享：百度大厦，地址：北京市海淀区上地十街10号，电话 010-59928888。http://j.map.baidu.com/BkmBk'
target/debug/backend --probe-shared-strict \
  'https://item.taobao.com/item.htm?id=752598787556' \
  '淘宝分享：复古实木桌面收纳盒，胡桃木色，中号，券后价 89 元。https://item.taobao.com/item.htm?id=752598787556'
target/debug/backend --probe-shared-strict \
  'https://detail.tmall.com/item.htm?id=821944906491' \
  '天猫分享：轻量防水通勤双肩包，黑色 20L，活动价 199 元。https://detail.tmall.com/item.htm?id=821944906491'
