#!/bin/sh

set -ex

BASE_COMMIT=$1
FUZZER_COMMIT=$2

apt update && apt -y install git zip curl clang python3-dev python3-tk

mkdir -p /ap/archipelago
cd /ap/archipelago

git init
git remote add origin https://github.com/Eijebong/Archipelago.git
git fetch origin ${BASE_COMMIT} --depth 1
git reset --hard ${BASE_COMMIT}

uv venv
uv pip install -r requirements.txt
uv pip install -r worlds/_sc2common/requirements.txt
uv pip install -r worlds/alttp/requirements.txt
uv pip install -r worlds/factorio/requirements.txt
uv pip install -r worlds/kh2/requirements.txt
uv pip install -r worlds/minecraft/requirements.txt
uv pip install -r worlds/sc2/requirements.txt
uv pip install -r worlds/soe/requirements.txt
uv pip install -r worlds/tloz/requirements.txt
uv pip install -r worlds/tww/requirements.txt
uv pip install -r worlds/zillion/requirements.txt
uv pip install -r worlds/zork_grand_inquisitor/requirements.txt
uv export --project=/ap/ap-worker/pyproject.toml --locked | uv pip install -r -
uv run cythonize -a -i _speedups.pyx
git rev-parse HEAD > /ap/archipelago/version
rm -Rf .git

mkdir -p /ap/supported_worlds
echo -e "jakanddaxter_options:\n  enforce_friendly_options: false" > /ap/archipelago/host.yaml

bash -ex /ap/prepare_worlds.sh /ap/archipelago /ap/supported_worlds/

mkdir /tmp/fuzzer
cd /tmp/fuzzer
git init
git remote add origin https://github.com/Eijebong/Archipelago-fuzzer.git
git fetch origin ${FUZZER_COMMIT} --depth 1
git reset --hard ${FUZZER_COMMIT}
cp fuzz.py /ap/archipelago/fuzz.py
ls -lah
cp -R hooks /ap/archipelago/
touch /ap/archipelago/hooks/__init__.py

ln -s /ap/ap-worker/check_wq.py /ap/archipelago/check_wq.py
ln -s /ap/ap-worker/gen_wq.py /ap/archipelago/gen_wq.py
ln -s /ap/ap-worker/self_check.py /ap/archipelago/self_check.py
ln -s /ap/ap-worker/ap_tests.py /ap/archipelago/ap_tests.py
