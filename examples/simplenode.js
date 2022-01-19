


const prtp = require( "projectrtp" )
const https = require( "https" )

function wgets( url ) {
  return new Promise( r => {
    https.get( url, res => {
      res.setEncoding( "utf8" )
      let body = ""
      res.on( "data", data => body += data )
      res.on( "end", () => {
        r( body.trim() )
      } )
    } )
  } )
}

async function connect() {
  let port = 9002
  let host = "127.0.0.1"
  let pa = "127.0.0.1"

  /* Public Address */
  if( undefined !== process.env.PA ) { 
    pa = process.env.PA
  } else {
    pa = await wgets( "https://checkip.amazonaws.com" )
  }

  prtp.projectrtp.setaddress( pa )

  if( undefined !== process.env.PORT ) port = parseInt( process.env.PORT )
  if( undefined !== process.env.HOST ) host = process.env.HOST

  prtp.projectrtp.proxy.connect( port, host )
}

connect()