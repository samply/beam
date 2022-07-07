#/bin/bash

eval $(cargo run --bin central -- examples test.broker.samply.de)

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

export BROKER_URL="http://localhost:8080"
export CLIENT_ID="test.broker.samply.de"
export PKI_ADDRESS="http://localhost:8200"
export PKI_APIKEY_FILE="$SCRIPT_DIR/tests/pki_apikey.secret"
export PRIVKEY_FILE="$SCRIPT_DIR/pki/test.priv.pem"
export CLIENTKEY_test="MySecret"

function do_curl_get {
    echo curl -v -H "content-type: application/json" -H "Authorization: ClientApiKey test MySecret" http://localhost:8081$@
}

function do_curl_post {
    DATA="$1"
    shift
    echo curl -v -H "content-type: application/json" --data "$DATA" http://localhost:8080$@
    curl -v -H "content-type: application/json" --data "$DATA" http://localhost:8080$@
}

function do_curl_put {
    DATA="$1"
    shift
    echo curl -v -H "content-type: application/json" -X PUT --data "$DATA" http://localhost:8080$@
    curl -v -H "content-type: application/json" -X PUT --data "$DATA" http://localhost:8080$@
}

function put_task {
    do_curl_post "$TASK0" /tasks
}

function put_results {
    do_curl_put "$RESULT0" /tasks
    do_curl_put "$RESULT1" /tasks
}

export -f do_curl_get do_curl_post put_task put_results

