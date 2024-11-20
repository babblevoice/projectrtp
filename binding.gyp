{
  "targets": [
    {
      "target_name": "projectrtp",
      "defines": [ "NODE_MODULE", "BOOST_NO_EXCEPTIONS", "BOOST_EXCEPTION_DISABLE", "NAPI_DISABLE_CPP_EXCEPTIONS" ],
      "cflags_cc!": [ "-fno-rtti" ],
      "cflags_cc": [
        "-O3",
        "-g",
        "-Wall",
        "-fstack-protector-strong",
        "-std=c++20",
        "-Weffc++" ],
      "ldflags": [
        "-Wl,-z,relro",
        "-Wl,-z,now",
        "-Wl,--export-dynamic" ],
      "libraries": [
            "-lrt",
            "-lspandsp",
            "-lilbc",
            "-lgnutls",
            "-lsrtp2" ],
      "sources": [
        "src/boostexception.cpp",
        "src/projectrtpfirfilter.cpp",
        "src/projectrtpnodemain.cpp",
        "src/projectrtpbuffer.cpp",
        "src/projectrtppacket.cpp",
        "src/projectrtprawsound.cpp",
        "src/projectrtpcodecx.cpp",
        "src/projectrtpchannelrecorder.cpp",
        "src/projectrtpsoundfile.cpp",
        "src/projectrtpsoundsoup.cpp",
        "src/projectrtptonegen.cpp",
        "src/projectrtpchannel.cpp",
        "src/projectrtpchannelmux.cpp",
        "src/projectrtpsrtp.cpp",
        "src/projectrtpstun.cpp" ],
      "include_dirs": [
        "src"
      ]
    }
  ]
}
