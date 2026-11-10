#!/bin/bash
# bwrap stand-in for the LoOS Linux layer: there are no namespaces or seccomp yet, so the program runs
# unsandboxed. Environment and working-directory options are honoured; mount/isolation options are skipped.
env_clear=0; dir=/; declare -a sets=()
while [ $# -gt 0 ]; do
    case $1 in
        --) shift; break ;;
        --clearenv) env_clear=1; shift ;;
        --setenv) sets+=("$2=$3"); shift 3 ;;
        --unsetenv) unset "$2"; shift 2 ;;
        --chdir) dir=$2; shift 2 ;;
        --ro-bind|--ro-bind-try|--bind|--bind-try|--dev-bind|--dev-bind-try|--symlink|--chmod) shift 3 ;;
        --dev|--proc|--tmpfs|--dir|--remount-ro|--seccomp|--add-seccomp-fd|--uid|--gid|--hostname|--file|--bind-data|--ro-bind-data|--lock-file|--sync-fd|--info-fd|--json-status-fd|--block-fd|--userns-block-fd|--cap-add|--cap-drop|--perms|--size|--args|--exec-label|--file-label|--argv0) shift 2 ;;
        --*) shift ;;
        *) break ;;
    esac
done
cd "$dir" || exit 1
if [ $env_clear = 1 ]; then exec env -i "${sets[@]}" "$@"; fi
exec env "${sets[@]}" "$@"
