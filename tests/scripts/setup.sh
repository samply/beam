#/bin/bash

eval $(cargo run --bin central -- examples test.broker.samply.de 2>/dev/null)

SD=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
export WORKSPACE=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && cd ../../ && pwd )

export BROKER_URL="http://localhost:8080"
export CLIENT_ID="test.broker.samply.de"
export PKI_ADDRESS="http://localhost:8200"
export PKI_APIKEY_FILE="$WORKSPACE/tests/pki_apikey.secret"
export PRIVKEY_FILE="$WORKSPACE/pki/test.priv.pem"
export CLIENTKEY_test="MySecret"

export P="http://localhost:8081" # for scripts

TARGET_DIR=$(cat ~/.cargo/config.toml | grep "^target-dir" |sed 's;.*\"\(.*\)\".*;\1;g')
if [ -z $TARGET_DIR ]; then
    TARGET_DIR=target
fi

cd $WORKSPACE

function start {
    pki/pki devsetup &
    while ! openssl rsa -in $PRIVKEY_FILE -noout -check >/dev/null 2>/dev/null; do 
        sleep 0.1
    done
    cargo build 2>/dev/null
    $TARGET_DIR/debug/proxy &
    $TARGET_DIR/debug/central &
}

function stop {
    killall vault proxy central
}

function clean {
    pki/pki clean
}

function testing {
    echo "TEST \"$@\""
}

function fail {
    echo "FAIL $@"
    exit 1
}

function success {
    echo "  OK"
}

function curl_get {
    curl -H "content-type: application/json" -H "Authorization: ApiKey test.$CLIENT_ID MySecret" $@
}

function curl_get_out {
    out="$1"
    shift
    curl_get -s -w %{"$out"} "$@"
}

function curl_get_noout {
    out="$1"
    shift
    curl_get_out "$out" -o /dev/null "$@"
}


function curl_post {
    curl -H "content-type: application/json" -H "Authorization: ClientApiKey test.$CLIENT_ID MySecret" -d @- $@
}

function curl_post_out {
    out="$1"
    shift
    curl_post -s -w %{"$out"} "$@"
}

function curl_post_noout {
    out="$1"
    shift
    curl_post_out "$out" -o /dev/null "$@"
}

export -f curl_get curl_get_out curl_get_noout curl_post curl_post_out curl_post_noout start stop clean testing fail success
