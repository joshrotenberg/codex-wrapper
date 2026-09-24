#!/usr/bin/env bash
# Fake `codex app-server` for the unit tests in src/app_server.rs.
#
# The message shapes are transcribed from captured codex-cli 0.149.0 sessions
# (stdio transport, one JSON object per line, no `jsonrpc` field). Ids, paths,
# and timestamps are synthetic; the shapes are not. The captured facts this
# relies on are listed in the module documentation of `app_server`.
#
# Behaviour is chosen with FAKE_APP_SERVER_MODE:
#
#   turn           a turn that completes by itself (default)
#   held           a turn that stays open until `turn/steer` or `turn/interrupt`
#   approval       a turn that stops on a server-initiated approval request
#   silent         never answers `initialize`
#   mute-after-init  answers `initialize`, then ignores every request
#   noisy-stderr   writes about 500 KB to stderr before answering `initialize`
#   exit-on-start  fails before serving, as the CLI does for an unknown `-c` key
#                  under --strict-config
#   exit-on-turn   exits without answering `turn/start`
#   flood          sends one very long notification after `initialized`
#   bad-json       sends a line that starts like JSON but is not
#   spawns-child   as `turn`, and records its pid and a child's, like
#                  fake-codex-spawns-child.sh. On SIGTERM it writes
#                  "$CODEX_WRAPPER_TEST_PIDFILE.term" and exits, so a test can
#                  tell a graceful stop from a kill.
#   ignores-eof    as `turn`, but keeps running after stdin closes, and
#                  handles SIGTERM like `spawns-child`
#
# FAKE_APP_SERVER_ITEMS=legacy makes `turn/completed` carry no items, as
# codex-cli 0.145.0 does.
#
# CODEX_WRAPPER_TEST_PIDFILE names the file `spawns-child` and `ignores-eof`
# write their pids to. FAKE_APP_SERVER_LOG, when set, receives every line the
# client sends, so a test can assert what is on the wire.

mode="${FAKE_APP_SERVER_MODE:-turn}"

THREAD=01a0d1a2-5f2e-7263-a97b-cabf3e78caff
TURN=01a0d1a2-5f9f-75c1-9305-fa382e7b37bc
CWD=/tmp/fixture

if [ "$mode" = exit-on-start ]; then
  echo 'Error: unknown configuration field `bogus_key` in -c/--config override' >&2
  exit 1
fi

if [ "$mode" = spawns-child ] || [ "$mode" = ignores-eof ]; then
  sleep 60 &
  child=$!
  {
    echo "parent=$$"
    echo "child=$child"
  } > "$CODEX_WRAPPER_TEST_PIDFILE"
fi
if [ "$mode" = spawns-child ] || [ "$mode" = ignores-eof ]; then
  trap 'touch "$CODEX_WRAPPER_TEST_PIDFILE.term"; exit 0' TERM
fi

THREAD_JSON='{"id":"'$THREAD'","extra":null,"sessionId":"'$THREAD'","forkedFromId":null,"parentThreadId":null,"preview":"","ephemeral":true,"section":null,"sectionEnteredAt":null,"projectId":null,"historyMode":"legacy","modelProvider":"openai","createdAt":1790223474,"updatedAt":1790223474,"recencyAt":1790223474,"status":{"type":"idle"},"path":null,"cwd":"'$CWD'","cliVersion":"0.149.0","source":"vscode","canAcceptDirectInput":true,"threadSource":null,"agentNickname":null,"agentRole":null,"gitInfo":null,"name":null,"turns":[]}'

emit() { printf '%s\n' "$1"; }
result() { emit '{"id":'"$1"',"result":'"$2"'}'; }
error() { emit '{"error":{"code":-32600,"message":"'"$2"'"},"id":'"$1"'}'; }
notify() { emit '{"method":"'"$1"'","params":'"$2"',"emittedAtMs":1790223474595}'; }

status_changed() {
  notify thread/status/changed '{"threadId":"'$THREAD'","status":'"$1"'}'
}

# The tail of every turn: usage, back to idle, and the terminal notification.
finish_turn() {
  # $1 status, $2 items json
  active=0
  notify thread/tokenUsage/updated '{"threadId":"'$THREAD'","turnId":"'$TURN'","tokenUsage":{"total":{"totalTokens":14585,"inputTokens":14524,"cachedInputTokens":7040,"cacheWriteInputTokens":0,"outputTokens":61,"reasoningOutputTokens":0},"last":{"totalTokens":14585,"inputTokens":14524,"cachedInputTokens":7040,"cacheWriteInputTokens":0,"outputTokens":61,"reasoningOutputTokens":0},"modelContextWindow":258400}}'
  status_changed '{"type":"idle"}'
  case "$1" in
    completed) itemsView=summary ;;
    *) itemsView=notLoaded ;;
  esac
  # codex-cli 0.145.0 never puts items in turn/completed.
  if [ "${FAKE_APP_SERVER_ITEMS:-}" = legacy ]; then
    itemsView=notLoaded
    set -- "$1" '[]'
  fi
  notify turn/completed '{"threadId":"'$THREAD'","turn":{"id":"'$TURN'","items":'"$2"',"itemsView":"'$itemsView'","status":"'$1'","error":null,"startedAt":1790223474,"completedAt":1790223492,"durationMs":17837}}'
}

agent_message() {
  # $1 text: emits the item and returns its json in $AGENT_ITEM
  local item='{"type":"agentMessage","id":"msg_03c16d5d9128d3ac016ab4a48435e487d0ac76257eac5001cc","text":"'"$1"'","phase":"final_answer","memoryCitation":null,"delivery":null}'
  AGENT_ITEM="$item"
  notify item/started '{"item":{"type":"agentMessage","id":"msg_03c16d5d9128d3ac016ab4a48435e487d0ac76257eac5001cc","text":"","phase":"final_answer","memoryCitation":null,"delivery":null},"threadId":"'$THREAD'","turnId":"'$TURN'","startedAtMs":1790223492225}'
  notify item/agentMessage/delta '{"threadId":"'$THREAD'","turnId":"'$TURN'","itemId":"msg_03c16d5d9128d3ac016ab4a48435e487d0ac76257eac5001cc","delta":"'"$1"'"}'
  notify item/completed '{"item":'"$item"',"threadId":"'$THREAD'","turnId":"'$TURN'","completedAtMs":1790223492412}'
}

complete_turn() {
  agent_message "$1"
  finish_turn completed "[$AGENT_ITEM]"
}

user_message() {
  # $1 text
  local item='{"type":"userMessage","id":"01a0d1a2-a0f6-72b3-9064-d0437f9a20cb","clientId":null,"content":[{"type":"text","text":"'"$1"'","text_elements":[]}]}'
  notify item/started '{"item":'"$item"',"threadId":"'$THREAD'","turnId":"'$TURN'","startedAtMs":1790223491318}'
  notify item/completed '{"item":'"$item"',"threadId":"'$THREAD'","turnId":"'$TURN'","completedAtMs":1790223491318}'
}

start_turn() {
  active=1
  result "$1" '{"turn":{"id":"'$TURN'","items":[],"itemsView":"notLoaded","status":"inProgress","error":null,"startedAt":null,"completedAt":null,"durationMs":null}}'
  status_changed '{"type":"active","activeFlags":[]}'
  notify turn/started '{"threadId":"'$THREAD'","turn":{"id":"'$TURN'","items":[],"itemsView":"notLoaded","status":"inProgress","error":null,"startedAt":1790223474,"completedAt":null,"durationMs":null}}'
}

request_re='^\{"id":(-?[0-9]+),"method":"([^"]*)"'
notification_re='^\{"method":"([^"]*)"'
reply_re='^\{"id":(-?[0-9]+),"result":'

initialized=0
awaiting_approval=0
active=0

while IFS= read -r line; do
  [ -n "${FAKE_APP_SERVER_LOG:-}" ] && printf '%s\n' "$line" >> "$FAKE_APP_SERVER_LOG"
  if [[ $line =~ $request_re ]]; then
    id=${BASH_REMATCH[1]}
    method=${BASH_REMATCH[2]}
  elif [[ $line =~ $reply_re ]]; then
    # The client answering a server-initiated request.
    if [ "$awaiting_approval" = 1 ]; then
      awaiting_approval=0
      decision=none
      [[ $line =~ \"decision\":\"([^\"]*)\" ]] && decision=${BASH_REMATCH[1]}
      notify serverRequest/resolved '{"threadId":"'$THREAD'","requestId":0}'
      complete_turn "decision: $decision"
    fi
    continue
  elif [[ $line =~ $notification_re ]]; then
    if [ "${BASH_REMATCH[1]}" = initialized ]; then
      initialized=1
      notify remoteControl/status/changed '{"status":"disabled","serverName":"host.local","installationId":"00000000-0000-0000-0000-000000000000","environmentId":null}'
      case "$mode" in
        flood)
          pad=$(printf 'x%.0s' $(seq 1 4000))
          notify noise '{"pad":"'"$pad"'"}'
          ;;
        bad-json) emit '{"method": oops' ;;
      esac
    fi
    continue
  else
    continue
  fi

  if [ "$method" = initialize ]; then
    if [ "$initialized" = 1 ]; then
      error "$id" "Already initialized"
      continue
    fi
    [ "$mode" = silent ] && continue
    if [ "$mode" = noisy-stderr ]; then
      for _ in $(seq 1 2000); do
        printf '\033[2m2026-09-24T04:17:33.188167Z\033[0m \033[31mERROR\033[0m \033[2mrmcp::transport::worker\033[0m\033[2m:\033[0m worker quit with fatal: Transport channel closed, when Client(HttpRequest(HttpRequest("http/request failed: error sending request for url (http://127.0.0.1:3001/)")))\n' >&2
      done
    fi
    result "$id" '{"userAgent":"codex-wrapper-test/0.149.0 (Mac OS 27.0.0; arm64) test (codex-wrapper-test; 0)","codexHome":"/home/user/.codex","platformFamily":"unix","platformOs":"macos"}'
    continue
  fi

  if [ "$initialized" != 1 ]; then
    error "$id" "Not initialized"
    continue
  fi
  [ "$mode" = mute-after-init ] && continue

  case "$method" in
    thread/start)
      result "$id" '{"thread":'"$THREAD_JSON"',"model":"gpt-5.6-sol","modelProvider":"openai","serviceTier":"default","cwd":"'$CWD'","runtimeWorkspaceRoots":["'$CWD'"],"instructionSources":[],"approvalPolicy":"never","approvalsReviewer":"auto_review","sandbox":{"type":"readOnly","networkAccess":false},"activePermissionProfile":null,"reasoningEffort":"medium","multiAgentMode":"explicitRequestOnly"}'
      notify thread/started '{"thread":'"$THREAD_JSON"'}'
      ;;
    turn/start)
      if [ "$mode" = exit-on-turn ]; then
        echo 'fatal: connection to the model provider was lost' >&2
        exit 5
      fi
      start_turn "$id"
      case "$mode" in
        held) : ;;
        approval)
          notify item/started '{"item":{"type":"commandExecution","id":"exec-2835e0b7-4c39-43f0-8407-ea5344c79846","pluginId":null,"scriptPath":null,"command":"/bin/zsh -lc '"'"'touch created.txt'"'"'","cwd":"'$CWD'","processId":null,"source":"agent","status":"inProgress","commandActions":[{"type":"unknown","command":"touch created.txt"}],"aggregatedOutput":null,"exitCode":null,"durationMs":null},"threadId":"'$THREAD'","turnId":"'$TURN'","startedAtMs":1790223503899}'
          emit '{"method":"item/commandExecution/requestApproval","id":0,"params":{"threadId":"'$THREAD'","turnId":"'$TURN'","itemId":"exec-2835e0b7-4c39-43f0-8407-ea5344c79846","startedAtMs":1790223503898,"environmentId":"local","command":"/bin/zsh -lc '"'"'touch created.txt'"'"'","cwd":"'$CWD'","commandActions":[{"type":"unknown","command":"touch created.txt"}],"proposedExecpolicyAmendment":["touch","created.txt"],"availableDecisions":["accept",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["touch","created.txt"]}},"cancel"]}}'
          awaiting_approval=1
          ;;
        *) complete_turn ok ;;
      esac
      ;;
    turn/steer)
      expected=""
      [[ $line =~ \"expectedTurnId\":\"([^\"]*)\" ]] && expected=${BASH_REMATCH[1]}
      if [ "$active" != 1 ]; then
        error "$id" "no active turn to steer"
      elif [ "$expected" != "$TURN" ]; then
        error "$id" "expected active turn id \`$expected\` but found \`$TURN\`"
      else
        result "$id" '{"turnId":"'$TURN'"}'
        text=""
        [[ $line =~ \"text\":\"([^\"]*)\" ]] && text=${BASH_REMATCH[1]}
        user_message "$text"
        complete_turn "DONE STEERED"
      fi
      ;;
    turn/interrupt)
      target=""
      [[ $line =~ \"turnId\":\"([^\"]*)\" ]] && target=${BASH_REMATCH[1]}
      if [ "$active" != 1 ]; then
        error "$id" "no active turn to interrupt"
      elif [ "$target" != "$TURN" ]; then
        error "$id" "expected active turn id $target but found $TURN"
      else
        result "$id" '{}'
        finish_turn interrupted '[]'
      fi
      ;;
    *)
      error "$id" "Invalid request: unknown variant \`$method\`, expected one of \`initialize\`, \`thread/start\`, \`turn/start\`"
      ;;
  esac
done

# stdin closed. The real server exits 0 here.
if [ "$mode" = ignores-eof ]; then
  wait "$child"
fi
exit 0
