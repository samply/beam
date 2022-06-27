#/bin/bash

eval $(cargo run --bin central -- examples)

function do_curl_get {
    echo curl -v -H "content-type: application/json" http://localhost:8080$@
    curl -v -H "content-type: application/json" http://localhost:8080$@
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