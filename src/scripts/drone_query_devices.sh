CAM_USB_VID="2085"
CAM_USB_PID="f37e"

# 1-6.1 Bus 1 / Port 6 / Port 1
readarray hub_vendor_match <<< $(grep -l ${CAM_USB_VID} /sys/bus/usb/devices/*/idVendor)
for match in ${hub_vendor_match[@]}
do
    echo ov5640 $(basename $(dirname ${match}))
done
