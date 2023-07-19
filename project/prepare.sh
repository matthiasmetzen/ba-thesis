#! /bin/sh

SERVER=$1

SIZES=("1KiB" "10KiB" "100KiB" "500KiB" "1MiB")

for size in "${SIZES[@]}"; do
    warp get --host $SERVER --bucket bench --prefix "${size,,}" --obj.size $size --access-key user --secret-key password --noclear --duration 0s --stress
done



printf "\n"