

# Testing and having a look around

podman run -it fedora-projectrtp:2021-06-16 /bin/bash

# Running

projectrtp is installed into /usr/bin

podman run -it fedora-projectrtp:2021-06-16 /usr/bin/projectrtp --fg --pa $(curl --silent http://checkip.amazonaws.com) --connect 127.0.0.1 --chroot /mnt/sounds/


# Upload

buildah --creds myaccount:mypassword push projectrtp:fedora-2021-06-16 tinpotnick/projectrtp:fedora-2021-06-16
