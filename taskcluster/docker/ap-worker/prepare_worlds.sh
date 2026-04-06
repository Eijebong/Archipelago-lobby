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

    (cd $(dirname $f) && zip -r ${DEST}/$(basename $f)-${VERSION}.apworld $(basename $f))
    rm -Rf "$f"
done
