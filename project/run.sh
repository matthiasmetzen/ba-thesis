#! /bin/sh

while getopts ':s:p:i:n:u:h' opt; do
  case "$opt" in
    s)
      arg="$OPTARG"
      SERVER="$OPTARG"
      ;;

    p)
      arg="$OPTARG"
      PORT="$OPTARG"
      ;;

    i)
      arg="$OPTARG"
      INTERFACE="$OPTARG"
      ;;

    n)
      arg="$OPTARG"
      NAME="$OPTARG"
      ;;

    u)
      arg="$OPTARG"
      UPSTREAM_PORT="$OPTARG"
      ;;
   
    : )
      echo "Invalid option: $OPTARG requires an argument" 1>&2
      exit 1
      ;;

    ?|h)
      echo "Usage: $(basename $0) [-s host] [-p port] [-n name] [-i interface] [-u port] [warp|oha]"
      exit 1
      ;;
  esac
done
shift "$(($OPTIND -1))"
subcommand=$1; shift # Get subcommand and shift to next option
case "$subcommand" in
    warp ) # pow chicka pow pow
        unset OPTIND
        RUN_WARP=true
    ;;
    oha ) # pow chicka pow pow
        unset OPTIND
        RUN_OHA=true
    ;;
    * ) # Invalid subcommand
        if [ ! -z $subcommand ]; then  # Don't show if no subcommand provided
            echo "Invalid subcommand: $subcommand"
            exit 1
        fi
        
        RUN_WARP=true
        RUN_OHA=true
    ;;
esac
        

SERVER=${SERVER:-127.0.0.1}
PORT=${PORT:-9000}
INTERFACE=${INTERFACE:-lo}
UPSTREAM_PORT=${UPSTREAM_PORT:-9000}
NAME=${NAME:-default}

SIZES=("1KiB" "10KiB" "100KiB" "500KiB" "1MiB")

mkdir -p "data/${NAME}"
mkdir -p "output/${NAME}"

### warp
run_warp() {
    echo "Running: warp"
    
    for size in "${SIZES[@]}"; do
        echo "Starting iftop in the background"
        nohup iftop -nNPt -i ${INTERFACE} -f "port ${UPSTREAM_PORT}" < /dev/null > "output/${NAME}/iftop-warp-${SERVER}:${PORT}-${size}.log" &

        echo "Running warp for size ${size}"
        filename="warp-${SERVER}:${PORT}-${size}"
        warp get --host "${SERVER}:${PORT}" --bucket bench --prefix ${size,,} --access-key user --secret-key password --list-existing --duration 2m --noclear --benchdata "data/${NAME}/${filename}"
        printf "\n"
        warp analyze --analyze.v "data/${NAME}/${filename}.csv.zst" > "output/${NAME}/${filename}.log"
        printf "\n"

        echo "Stopping iftop..."
        # let iftop settle
        sleep 30
        # stop iftop
        kill -9 % && wait % 2> /dev/null
	sleep 120
    done
}

### oha
run_oha() {
    echo "Running: oha"
    
    for size in "${SIZES[@]}"; do 
        echo "Starting iftop in the background"
        nohup iftop -nNPt -i ${INTERFACE} -f "port ${UPSTREAM_PORT}" < /dev/null > "output/${NAME}/iftop-oha-${SERVER}:${PORT}-${size}.log" &

        echo "Running oha for size ${size}"
        filename="oha-${SERVER}:${PORT}-${size}"
        OBJECT=$(curl -s "http://${SERVER}:${PORT}/bench/?prefix=${size,,}&max-keys=1" | xpath -q -e 'ListBucketResult/Contents/Key[1]/text()')
        oha "http://${SERVER}:${PORT}/bench/${OBJECT}" -z 1m > "output/${NAME}/${filename}.log"

        echo "Stopping iftop..."
        # let iftop settle
        sleep 30
        # stop iftop
        kill -9 % && wait % 2> /dev/null
	sleep 120
    done
}

if [ ! -z "$RUN_WARP" ]; then
    run_warp
fi

if [ ! -z "$RUN_OHA" ]; then
    run_oha
fi
