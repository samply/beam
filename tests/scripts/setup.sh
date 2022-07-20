#/bin/bash

SD=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
export WORKSPACE=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && cd ../../ && pwd )

export BROKER_URL="http://localhost:8080"
export BROKER_ID="localhost"
export PROXY_ID="proxy23.localhost"
export PKI_ADDRESS="http://localhost:8200"
export PKI_APIKEY_FILE="$WORKSPACE/tests/pki_apikey.secret"
export PRIVKEY_FILE="$WORKSPACE/pki/proxy23.priv.pem"

export APP_0_ID="testing"
export APP_0_KEY="TestSecret"

export APP_1_ID="pusher1"
export APP_1_KEY="Pusher1Secret"

export APP_2_ID="pusher2"
export APP_2_KEY="Pusher2Secret"

export P="http://localhost:8081" # for scripts

source $SD/example_json.sh

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
    if [ -x $TARGET_DIR/release/proxy ]; then
        $TARGET_DIR/release/proxy &
        $TARGET_DIR/release/broker &
    else
        cargo build
        $TARGET_DIR/debug/proxy &
        $TARGET_DIR/debug/broker &
    fi
}

function stop {
    killall vault proxy broker
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
    curl -H "content-type: application/json" -H "Authorization: ApiKey $APP_0_ID.$PROXY_ID $APP_0_KEY" $@
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
    curl -H "content-type: application/json" -H "Authorization: ClientApiKey $APP_0_ID.$PROXY_ID $APP_0_KEY" -d @- $@
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
