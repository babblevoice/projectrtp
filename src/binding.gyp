{
  "targets": [
    {
      "target_name": "projectrtp",
      "defines": [ "NODE_MODULE", "BOOST_NO_EXCEPTIONS" ],
      "cflags_cc": [
        "-O3",
        "-Wall",
        "-fstack-protector-all",
        "-std=c++17",
        "-fconcepts-ts",
        "-Weffc++" ],
      "libraries": [
            "-lrt",
            "-lspandsp",
            "-lilbc",
            "-lgnutls",
            "-lsrtp2",
            "-fstack-protector-all"
          ],
      "sources": [
        "projectrtpfirfilter.cpp",
        "projectrtpnodemain.cpp",
        "projectrtpbuffer.cpp",
        "projectrtppacket.cpp",
        "projectrtprawsound.cpp",
        "projectrtpcodecx.cpp",
        "projectrtpchannelrecorder.cpp",
        "projectrtpsoundfile.cpp",
        "projectrtpsoundsoup.cpp",
        "projectrtptonegen.cpp",
        "projectrtpchannel.cpp",
        "projectrtpchannelmux.cpp",
        "projectrtpsrtp.cpp",
        "projectrtpstun.cpp" ]
    }
  ]
}
