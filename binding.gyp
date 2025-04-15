{
  "variables": {
    "build_type%": "prod"
  },
  "targets": [
    {
      "target_name": "projectrtp",
      "defines": [ "NODE_MODULE", "BOOST_NO_EXCEPTIONS", "BOOST_EXCEPTION_DISABLE", "NAPI_DISABLE_CPP_EXCEPTIONS" ],
      "cflags_cc!": [ "-fno-rtti" ],
      "cflags_cc": [
        "-g",
        "-std=c++20",
        "-Weffc++" ],
      "conditions": [
        [
          "build_type=='dev'", {
            "cflags": [
              "-O1",
              "-fsanitize=address,undefined,bounds",
              "-fno-omit-frame-pointer",
              "-D_FORTIFY_SOURCE=2",
              "-fstack-protector-strong",
              "-fstack-protector-all",
              "-Wformat-security",
              "-Wall",
              "-Wextra"
            ],
            "ldflags": [
              "-fsanitize=address,undefined,bounds"
            ]
          }
        ],
        [
          "build_type=='prod'", {
            "cflags": [
              "-O3",
              "-g"
            ]
          }
        ]
      ],
      "ldflags": [
        "-g",
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
