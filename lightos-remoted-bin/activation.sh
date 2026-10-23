#!/usr/bin/env bash
# Caller supplies validated runtime/config/service variables.
# shellcheck disable=SC2154

activation_systemctl() { timeout --foreground --kill-after=3s 45s systemctl "$@"; }
activation_systemd_run() { timeout --foreground --kill-after=3s 45s systemd-run "$@"; }
activation_sleep() { sleep 1; }

activation_doctor() {
    activation_systemd_run --quiet --wait --pipe --collect \
        --property="User=$runtime_user" \
        --property="SupplementaryGroups=video render" \
        --property="EnvironmentFile=$environment" \
        --property="EnvironmentFile=-$native_environment" \
        --property=RuntimeDirectory=lightos-remoted-doctor \
        --property=RuntimeDirectoryMode=0700 \
        --property=Environment=XDG_RUNTIME_DIR=/run/lightos-remoted-doctor \
        --property=RuntimeMaxSec=40s --property=TimeoutStopSec=3s \
        -- "$release/lightos-remoted" doctor
}

activation_status() {
    local properties key value
    properties=$(activation_systemctl show --property=ActiveState --property=SubState \
        --property=MainPID --property=ExecMainPID --property=NRestarts "$service") || return 1
    active_state='' sub_state='' main_pid='' exec_pid='' restart_count=''
    while IFS='=' read -r key value; do
        case $key in
            ActiveState) active_state=$value ;;
            SubState) sub_state=$value ;;
            MainPID) main_pid=$value ;;
            ExecMainPID) exec_pid=$value ;;
            NRestarts) restart_count=$value ;;
        esac
    done <<< "$properties"
    [[ $active_state == active && $sub_state == running &&
        $main_pid =~ ^[1-9][0-9]*$ && $exec_pid == "$main_pid" &&
        $restart_count =~ ^[0-9]+$ ]]
}

activation_probe() {
    local expected_pid=$1
    activation_systemd_run --quiet --wait --pipe --collect \
        --property="User=$runtime_user" \
        --property="SupplementaryGroups=video render" \
        --property="EnvironmentFile=$environment" \
        --property="EnvironmentFile=-$native_environment" \
        --property=RuntimeMaxSec=40s --property=TimeoutStopSec=3s \
        -- "$release/lightos-remoted" probe \
        --expect-pid "$expected_pid"
}

activation_unchanged() {
    local expected_pid=$1 expected_restarts=$2
    activation_status && [[ $exec_pid == "$expected_pid" && $restart_count == "$expected_restarts" ]]
}

activate_candidate() {
    local previous_pid expected_pid expected_restarts _sample
    previous_pid=$(activation_systemctl show --property=ExecMainPID --value "$service") || return 1
    [[ $previous_pid =~ ^[0-9]+$ ]] || return 1
    activation_systemctl daemon-reload || return 1
    activation_systemctl restart "$service" || return 1
    activation_status || return 1
    expected_pid=$exec_pid
    expected_restarts=$restart_count
    [[ $expected_pid != "$previous_pid" ]] || return 1
    activation_probe "$expected_pid" || return 1
    activation_unchanged "$expected_pid" "$expected_restarts" || return 1
    for _sample in 1 2 3 4 5; do
        activation_sleep || return 1
        activation_unchanged "$expected_pid" "$expected_restarts" || return 1
    done
    activation_probe "$expected_pid" || return 1
    activation_unchanged "$expected_pid" "$expected_restarts"
}

recover_candidate() {
    local recovery_status=0
    activation_systemctl stop "$service" || recovery_status=1
    if ! restore; then
        printf '%s\n' 'RECOVERY FAILED: disk restoration failed; candidate remains stopped; retain backup for manual recovery.' >&2
        return 1
    fi
    activation_systemctl daemon-reload || return 1
    if ((was_active)); then
        activation_systemctl restart "$service" || return 1
        activation_status || return 1
        local expected_pid=$exec_pid expected_restarts=$restart_count _sample
        for _sample in 1 2 3 4 5; do
            activation_sleep || return 1
            activation_unchanged "$expected_pid" "$expected_restarts" || return 1
        done
    fi
    return "$recovery_status"
}
