

function reset() {
   # set gpios back to inputs and unexport
   for index in `seq 0 0`
   do
      # turn off the red leds
      echo 0 > /sys/class/leds/pca963x:arm${index}:red/brightness
   done
   kill -TERM $(jobs -p)
   exit 0
}

for index in `seq 0 0`
do
   # turn on the red leds
   echo 32 > /sys/class/leds/pca963x:arm${index}:red/brightness
done
trap reset SIGTERM
sleep infinity &
wait
