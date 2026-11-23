#!/bin/sh
# 외부 process 플러그인 fixture(XXI.5.1·XXI.5.2). 첫 인자가 동작을 고릅니다.
#   well-behaved: host.info를 요청하고 heartbeat에 답하며 받은 응답을 $2 파일에 적습니다.
#   crash: 요청 하나를 보낸 뒤 code 3으로 끝납니다.
#   hang: stdin을 읽지 않고 멈춥니다(heartbeat 응답 없음).
#   garbage: JSON이 아닌 줄을 보냅니다.
mode="$1"
out="$2"
case "$mode" in
  hang)
    exec sleep 60
    ;;
  garbage)
    echo 'not json'
    exec sleep 60
    ;;
esac
echo '{"stellar_rpc":1,"id":1,"method":"host.info"}'
if [ "$mode" = "crash" ]; then
  exit 3
fi
while IFS= read -r line; do
  case "$line" in
    *'"type":"heartbeat"'*)
      seq=$(printf '%s' "$line" | sed 's/.*"seq":\([0-9]*\).*/\1/')
      echo "{\"stellar_process\":1,\"type\":\"heartbeat-ack\",\"seq\":$seq}"
      echo "heartbeat $seq" >> "$out"
      ;;
    *'"result"'*)
      echo "$line" >> "$out"
      ;;
  esac
done
