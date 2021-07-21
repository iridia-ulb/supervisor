

function release_gpios() {
   # set gpios back to inputs and unexport
   for index in `seq 0 3`
   do
      # set gpios as input
      echo "in" > /sys/class/gpio/gpio$(($gpiobase + $index))/direction
      # export the gpios
      echo $(($gpiobase + $index)) > /sys/class/gpio/unexport
   done
   kill -TERM $(jobs -p)
   exit 0
}

# set the id to 15 by default (an invalid value)
id=${1:-15}

# check if the id is in range
if ((${id} >= 0 && ${id} < 15)); then
   gpiobase=`grep -lr "of:NgpioTCnxp,pca9554" /sys/class/gpio/*/device/modalias | grep -o [0-9]*`
   for index in `seq 0 3`
   do
      # export the gpios
      echo $(($gpiobase + $index)) > /sys/class/gpio/export
      # set gpios as output
      echo "out" > /sys/class/gpio/gpio$(($gpiobase + $index))/direction
      # set the value
      if (( (id & (1 << $index)) != 0 )); then
         echo "1" > /sys/class/gpio/gpio$(($gpiobase + $index))/value
      else
         echo "0" > /sys/class/gpio/gpio$(($gpiobase + $index))/value
      fi
   done
   trap release_gpios SIGTERM
   sleep infinity &
   wait
else
  exit 1
fi





