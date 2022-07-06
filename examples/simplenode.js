


const prtp = require( "@babblevoice/projectrtp" )
const https = require( "https" )
const fs = require( "fs" )
prtp.projectrtp.run()

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

function createuktones( dir = "/tmp/" ) {
  /*
    350+440*0.5:1000 Dial tone (1S)
    400+4500.5/0/400+4500.5/0:400/200/400/2000 Ringing (1S-4S)
    697+12090.5/0/697+13360.5/0/697+14770.5/0/697+16330.5/0:400/100 DTMF 123A (400mS on 100mS off each tone 4S-12S)
    770+12090.5/0/770+13360.5/0/770+14770.5/0/770+16330.5/0:400/100 DTMF 456B
    852+12090.5/0/852+13360.5/0/852+14770.5/0/852+16330.5/0:400/100 DTMF 789C
    941+12090.5/0/941+13360.5/0/941+14770.5/0/941+16330.5/0:400/100 DTMF *0#D
    440:1000 Unobtainable (12S-13S)
    440/0:375/375 Busy (13S-13.75S)
    440/0:400/350/225/525 Congestion (13.75S-15.25S)
    440/0:125/125 Pay (15.25S-15.5S)
  */
  let filename = dir + "uksounds.wav"

  if( !fs.existsSync( filename ) ) {
    prtp.projectrtp.tone.generate( "350+440*0.5:1000", filename )
    prtp.projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", filename )
    prtp.projectrtp.tone.generate( "697+1209*0.5/0/697+1336*0.5/0/697+1477*0.5/0/697+1633*0.5/0:400/100", filename )
    prtp.projectrtp.tone.generate( "770+1209*0.5/0/770+1336*0.5/0/770+1477*0.5/0/770+1633*0.5/0:400/100", filename )
    prtp.projectrtp.tone.generate( "852+1209*0.5/0/852+1336*0.5/0/852+1477*0.5/0/852+1633*0.5/0:400/100", filename )
    prtp.projectrtp.tone.generate( "941+1209*0.5/0/941+1336*0.5/0/941+1477*0.5/0/941+1633*0.5/0:400/100", filename )
    prtp.projectrtp.tone.generate( "440:1000", filename )
    prtp.projectrtp.tone.generate( "440/0:375/375", filename )
    prtp.projectrtp.tone.generate( "440/0:400/350/225/525", filename )
    prtp.projectrtp.tone.generate( "440/0:125/125", filename )
  }
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
  console.log( "RTP Server running" )
}

createuktones()
connect()