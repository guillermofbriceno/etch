#!/usr/bin/env bash
# Version helpers shared by the release workflows.
#
#   version.sh check <version>            exit non-zero unless <version> is valid semver
#   version.sh release <version> <ref>    validate <version> as the next release from <ref>
#   version.sh dev <run-number> <sha>     print an automatic version for an untagged test build
#   version.sh msi <version> <run-number> print the numeric version WiX needs for the MSI
set -euo pipefail
export LC_ALL=C

ident='0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*'
semver_re="^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-($ident)(\.($ident))*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$"

fail() {
    echo "::error::$*" >&2
    exit 1
}

is_semver() {
    [[ $1 =~ $semver_re ]]
}

# Prints 1, 0 or -1 for the semver precedence of $1 against $2, ignoring build metadata.
semver_cmp() {
    local a=${1%%+*} b=${2%%+*}
    local ap='' bp='' i
    if [[ $a == *-* ]]; then ap=${a#*-}; fi
    if [[ $b == *-* ]]; then bp=${b#*-}; fi

    local IFS=.
    local -a x=(${a%%-*}) y=(${b%%-*})
    for i in 0 1 2; do
        if ((x[i] > y[i])); then echo 1; return; fi
        if ((x[i] < y[i])); then echo -1; return; fi
    done

    if [[ -z $ap && -z $bp ]]; then echo 0; return; fi
    if [[ -z $ap ]]; then echo 1; return; fi
    if [[ -z $bp ]]; then echo -1; return; fi

    local -a p=($ap) q=($bp)
    local s t
    for ((i = 0; i < ${#p[@]} && i < ${#q[@]}; i++)); do
        s=${p[i]} t=${q[i]}
        if [[ $s == "$t" ]]; then continue; fi
        if [[ $s =~ ^[0-9]+$ && $t =~ ^[0-9]+$ ]]; then
            if ((s > t)); then echo 1; else echo -1; fi
        elif [[ $s =~ ^[0-9]+$ ]]; then
            echo -1
        elif [[ $t =~ ^[0-9]+$ ]]; then
            echo 1
        elif [[ $s > $t ]]; then
            echo 1
        else
            echo -1
        fi
        return
    done

    if ((${#p[@]} > ${#q[@]})); then echo 1
    elif ((${#p[@]} < ${#q[@]})); then echo -1
    else echo 0
    fi
}

latest_tag() {
    local best='' tag v
    while read -r tag; do
        v=${tag#v}
        is_semver "$v" || continue
        if [[ -z $best || $(semver_cmp "$v" "$best") == 1 ]]; then best=$v; fi
    done < <(git tag -l 'v*')
    echo "$best"
}

cmd_check() {
    is_semver "$1" || fail "'$1' is not a valid version. Use the form 1.2.3 or 1.2.3-beta.1, without the leading v."
}

cmd_release() {
    local v=$1 ref=$2 latest
    cmd_check "$v"
    [[ $v != *+* ]] || fail "Release versions cannot carry build metadata (the part after +)."
    if git rev-parse -q --verify "refs/tags/v$v" >/dev/null; then
        fail "Tag v$v already exists."
    fi
    latest=$(latest_tag)
    if [[ -n $latest && $(semver_cmp "$v" "$latest") != 1 ]]; then
        fail "v$v is not newer than the latest tag, v$latest."
    fi
    if [[ $v != *-* && $ref != refs/heads/main ]]; then
        fail "Stable versions can only be released from main. Release a prerelease such as $v-beta.1 from other branches."
    fi
}

cmd_dev() {
    local run=$1 sha=${2:0:7} latest base major minor patch
    latest=$(latest_tag)
    latest=${latest:-0.0.0}
    latest=${latest%%+*}
    if [[ $latest == *-* ]]; then
        base=${latest%%-*}
    else
        IFS=. read -r major minor patch <<<"$latest"
        base="$major.$minor.$((patch + 1))"
    fi
    echo "$base-dev.$run+$sha"
}

# The MSI format only holds major.minor.patch.build, so prereleases use the run number as the build field.
cmd_msi() {
    local v=$1 run=$2 core major minor patch
    cmd_check "$v"
    core=${v%%[-+]*}
    IFS=. read -r major minor patch <<<"$core"
    if ((major > 255 || minor > 255 || patch > 65535)); then
        fail "$v cannot be expressed as an MSI version."
    fi
    if [[ $v == "$core" ]]; then
        echo "$core"
        return
    fi
    ((run <= 65535)) || fail "Run number $run is too large for the MSI build field."
    echo "$core.$run"
}

case ${1:-} in
check) cmd_check "$2" ;;
release) cmd_release "$2" "$3" ;;
dev) cmd_dev "$2" "$3" ;;
msi) cmd_msi "$2" "$3" ;;
*) fail "Usage: version.sh check|release|dev|msi ..." ;;
esac
