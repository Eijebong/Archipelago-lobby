set -ex

if [ "$1" == ""  ] || [ "$2" == "" ]; then
    echo "Usage: prepare_worlds.sh ap_root dest"
    exit 1
fi

DEST=$2
VERSION=$(cat $1/Utils.py | grep "__version__ =" | sed 's/__version__ = "\([0-9]\+.[0-9]\+.[0-9]\+\)"/\1/')

for f in $1/worlds/*; do
    if [[ -f $f ]]; then
        continue
    fi

    if [[ "$(basename $f)" == _* ]]; then
        continue;
    fi

    if [[ "$(basename $f)" == "generic" ]]; then
        continue
    fi

    # Until this gets fixed, alttp is essential for most things to work...
    # Yes it makes no sense, no I can't do anything about it.
    if [[ "$(basename $f)" == "alttp" ]]; then
        continue
    fi

    # FF1 throws errors when loaded as a .apworld
    if [[ "$(basename $f)" == "ff1" ]]; then
        continue
    fi

    # OoT throws errors when loaded as a .apworld
    if [[ "$(basename $f)" == "oot" ]]; then
        continue
    fi

    # Raft throws errors when loaded as a .apworld
    if [[ "$(basename $f)" == "raft" ]]; then
        continue
    fi

    # Lufia2AC throws errors when loaded as a .apworld
    if [[ "$(basename $f)" == "lufia2ac" ]]; then
        continue
    fi

    # SM throws errors when trying to get presets as a .apworld
    if [[ "$(basename $f)" == "sm" ]]; then
        continue
    fi

    (cd $(dirname $f) && zip -r ${DEST}/$(basename $f)-${VERSION}.apworld $(basename $f))
    rm -Rf "$f"
done
