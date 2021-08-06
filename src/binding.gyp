{
  "targets": [
    {
      "target_name": "projectrtp",
      "defines": [ "NODE_MODULE" ],
      "cflags_cc": [
        "-O3",
        "-Wall",
        "-fstack-protector-all",
        "-std=c++17",
        "-fconcepts-ts" ],
      "sources": [
        "firfilter.cpp",
        "projectrtpnodemain.cpp",
        "projectrtpbuffer.cpp",
        "projectrtppacket.cpp",
        "projectrtprawsound.cpp",
        "projectrtpcodecx.cpp",
        "projectrtpchannelrecorder.cpp",
        "projectrtpsoundfile.cpp",
        "projectrtpsoundsoup.cpp",
        "projectrtptonegen.cpp",
        "projectrtpchannel.cpp" ]
    }
  ]
}
