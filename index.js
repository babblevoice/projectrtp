
/* TODO */

/* */
if( process.platform == "win32" && process.arch == "x64" ) {
  throw "Platform not currently supported"
} else if( process.platform == "win32" && process.arch == "ia32" ) {
  throw "Platform not currently supported"
} else {
	module.exports.projectrtp = require( "./src/build/Release/projectrtp" )
}
