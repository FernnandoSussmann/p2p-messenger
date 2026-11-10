#!/usr/bin/env bash
# Level 3 smoke test: two throwaway headless nodes in Docker (see docker/compose.yaml).
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

PROJECT=p2pm-harness
COMPOSE=(docker compose -f docker/compose.yaml)

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

CURRENT=setup
trap 'echo "FAIL  $CURRENT"' ERR

stage() {
  CURRENT=$1
  shift
  "$@"
  echo "PASS  $CURRENT"
}

ctl() {
  local svc=$1
  shift
  "${COMPOSE[@]}" exec -T "$svc" messenger-headless ctl --data-dir /data "$@"
}

# Polls a command until it succeeds or the timeout (seconds) elapses.
poll() {
  local timeout=$1
  shift
  local end=$((SECONDS + timeout))
  until "$@" >/dev/null 2>&1; do
    if ((SECONDS >= end)); then return 1; fi
    sleep 1
  done
}

json_field() { sed -n "s/.*\"$1\":\"\\([^\"]*\\)\".*/\\1/p"; }

peers_lists() { ctl "$1" peers | grep -q "$2"; }
peers_lacks() { if peers_lists "$1" "$2"; then return 1; fi; }
wait_ctl_ready() { ctl "$1" node-id; }

stage_up() {
  "${COMPOSE[@]}" up -d --build
  poll 60 wait_ctl_ready node-a
  poll 60 wait_ctl_ready node-b
}

stage_lan() {
  A_ID=$(ctl node-a node-id | json_field node_id)
  [ -n "$A_ID" ]
  poll 60 peers_lists node-b "$A_ID"
}

stage_ticket() {
  local ticket
  ticket=$(ctl node-a ticket | json_field ticket)
  [ -n "$ticket" ]
  ctl node-b connect "$ticket" | grep -q '"ok":true'
  ctl node-a wait incoming_request --node "$B_ID" --timeout-secs 30 >/dev/null
  ctl node-a respond "$B_ID" accept | grep -q '"ok":true'
  ctl node-a wait connection_state --node "$B_ID" --state connected --timeout-secs 30 >/dev/null
  ctl node-b wait connection_state --node "$A_ID" --state connected --timeout-secs 30 >/dev/null
}

# Both nodes restart so node-a's event ring buffer is empty and a missing
# incoming_request is meaningful; both volumes keep identity and known peers.
stage_known_reconnect() {
  "${COMPOSE[@]}" restart node-a node-b
  poll 60 wait_ctl_ready node-a
  poll 60 wait_ctl_ready node-b
  [ "$(ctl node-b node-id | json_field node_id)" = "$B_ID" ]
  [ "$(ctl node-a node-id | json_field node_id)" = "$A_ID" ]
  local ticket
  ticket=$(ctl node-a ticket | json_field ticket)
  ctl node-b connect "$ticket" | grep -q '"ok":true'
  ctl node-a wait connection_state --node "$B_ID" --state connected --timeout-secs 30 >/dev/null
  if ctl node-a wait incoming_request --node "$B_ID" --timeout-secs 5 >/dev/null 2>&1; then
    echo "known peer produced an incoming_request" >&2
    return 1
  fi
}

stage_presence() {
  local t0=$SECONDS
  ctl node-a presence off | grep -q '"enabled":false'
  poll 180 peers_lacks node-b "$A_ID"
  echo "      A disappeared from B's peers after $((SECONDS - t0))s"
  ctl node-a presence on | grep -q '"enabled":true'
  poll 60 peers_lists node-b "$A_ID"
}

data_listing() { "${COMPOSE[@]}" exec -T "$1" ls -R /data; }
account_id() { "${COMPOSE[@]}" exec -T "$1" cat /data/account_id | tr -d '[:space:]'; }

stage_isolation() {
  [ "$A_ID" != "$B_ID" ]
  local a_acc b_acc
  a_acc=$(account_id node-a)
  b_acc=$(account_id node-b)
  [ -n "$a_acc" ]
  [ -n "$b_acc" ]
  [ "$a_acc" != "$b_acc" ]
  data_listing node-b | grep -q "$b_acc"
  if data_listing node-b | grep -q "$a_acc"; then return 1; fi
  if data_listing node-a | grep -q "$b_acc"; then return 1; fi
  local before
  before=$(data_listing node-b | sha256sum)
  "${COMPOSE[@]}" rm -s -f -v node-a
  [ "$("${COMPOSE[@]}" ps -q --status running node-b | wc -l)" -eq 1 ]
  [ "$(ctl node-b node-id | json_field node_id)" = "$B_ID" ]
  [ "$(data_listing node-b | sha256sum)" = "$before" ]
}

stage_teardown() {
  "${COMPOSE[@]}" down -v --remove-orphans
  [ -z "$(docker volume ls -q --filter "label=com.docker.compose.project=$PROJECT")" ]
  [ -z "$(docker ps -a -q --filter "label=com.docker.compose.project=$PROJECT")" ]
}

A_ID=
B_ID=

stage "1 up"               stage_up
A_ID=$(ctl node-a node-id | json_field node_id)
B_ID=$(ctl node-b node-id | json_field node_id)
stage "2 LAN discovery"    stage_lan
stage "3 ticket connect"   stage_ticket
stage "4 known reconnect"  stage_known_reconnect
stage "5 presence"         stage_presence
stage "6 isolation"        stage_isolation
stage "7 teardown"         stage_teardown
