
const { v4: uuidv4 } = require( "uuid" )
const EventEmitter = require( "events" )

const server = require( "./lib/server.js" )
const node = require( "./lib/node.js" )

const fs = require( "fs" )
const { spawnSync } = require( "child_process" )

let localaddress = "127.0.0.1"
let privateaddress = "127.0.0.1"
const bin = "./src/build/Release/projectrtp"


/**
 * Generate a self signed if none present
 * @return { void }
 * @ignore
 */
function gencerts() {

  const keypath = require( "os" ).homedir() + "/.projectrtp/certs/"
  if( !fs.existsSync( keypath + "dtls-srtp.pem" ) ) {

    if ( !fs.existsSync( keypath ) ) fs.mkdirSync( keypath, { recursive: true } )
    
    const serverkey = keypath + "server-key.pem"
    const servercsr = keypath + "server-csr.pem"
    const servercert = keypath + "server-cert.pem"
    const combined = keypath + "dtls-srtp.pem"

    const openssl = spawnSync( "openssl", [ "genrsa", "-out", serverkey, "4096" ] )
    if( 0 !== openssl.status ) throw new Error( "Failed to genrsa: " + openssl.status )

    const request = spawnSync( "openssl", [ "req", "-new", "-key", serverkey , "-out", servercsr, "-subj", "/C=GB/CN=projectrtp" ] )
    if( 0 !== request.status ) throw new Error( "Failed to generate csr: " + request.status )

    const sign = spawnSync( "openssl", [ "x509", "-req", "-in", servercsr, "-signkey", serverkey, "-out", servercert ] )
    if( 0 !== sign.status ) throw new Error( "Failed to sign key: " + sign.status )

    const serverkeydata = fs.readFileSync( serverkey )
    const servercertdata = fs.readFileSync( servercert )
    fs.writeFileSync( combined, Buffer.concat( [ serverkeydata, servercertdata ] ) )
    fs.unlinkSync( serverkey )
    fs.unlinkSync( servercsr )
    fs.unlinkSync( servercert )
    /* we will be left with combined */
  }
}

/**
 * Proxy for other RTP nodes - to be retired as it is ambiguous of direction (i.e. server/node). See node.interface and server.interface instead.
 */
class proxy {

  /**
   * @param { node.interface } ournode 
   * @param { server.interface } ourserver
   * @hideconstructor
   */
  constructor( ournode, ourserver ) {

    /** @private */
    this._node = ournode
    /** @private  */
    this._server = ourserver
  }

  /**
   * @summary Listen for connections from RTP nodes which can offer their services
   * to us. When we listen for other nodes, we can configure them so that it is invisible
   * to the main node as to where the channel is being handled.
   * @param { Object } port - port to listen on
   * @param { string } address - what address to listen to on
   * @param { object } em - event emitter
   * @return { Promise< server.rtpserver > }
   */
  async listen( em, address = "127.0.0.1", port = 9002 ) {
    return await this._server.listen( port, address, em )
  }

  /**
  @summary Listen for connections from RTP nodes which can offer their services
  to us. When we listen for other nodes, we can configure them so that it is invisible
  to the main node as to where the channel is being handled.
  @return { object }
  */
  stats() {
    return {
      "server": this._server.stats(),
      "node": {}
    }
  }

  /**
  @summary Returns details of all of the nodes connected to us.
  @return { Object }
  */
  nodes() {
    return this._server.nodes()
  }

  /**
   * We are a node and get the connection object.
   * @returns { node.rtpnode }
   */
  node() {
    return this._node.get()
  }

  /**
   * @summary Connect node to rtp srvers listening.
   * @param {number} port
   * @param {string} host
   * @return { Promise< node.rtpnode > }
   */
  connect( port = 9002, host = "127.0.0.1" ) {
    return this._node.connect( port, host )
  }

  /**
   * @param { object } node - object contain port and host
   * @param { string } node.host - host name
   * @param { number } node.port - port to connect to
   */
  addnode( node ) {
    return this._server.addnode( node )
  }

  /**
   * Clear current list of nodes (nodes configured for listening)
   */
  clearnodes() {
    this._server.clearnodes()
  }

  get () {
    return server.interface.get()
  }
}

/**
 * Callback for events we pass back to interested parties.
 * @callback channelcallback
 * @param {object} event
 * @listens close
 * @listens record
 * @listens play
 * @listens telephone-event
 */

/**
 * Channel closed event
 * @event close
 * @type {object}
 * @property {string} action - "close"
 * @property {string} reason - the reason the channel was closed
 * @property {object} tick - statistics related to the channels interval timer
 * @property {object} in - in traffic statistics
 * @property {object} out - out traffic statistics
 */

/**
 * Channel recording events
 * @event record
 * @type {object}
 * @property {string} action - "record"
 * @property {string} file - filename of the recording
 * @property {string} event - details of what happened
 */

/**
 * Events related to sound file playback
 * @event play
 * @type {object}
 * @property {string} action - "play"
 * @property {string} event - what the event was
 * @property {string} reason - more details regarding the event
 */

/**
 * RFC 2833 telephone-event
 * @event telephone-event
 * @type {object}
 * @property {string} action - "telephone-event"
 * @property {string} event - the DTMF character pressed
 */

/**
 * @typedef { Object } channel
 * @property { function } close
 * @property { function } remote
 * @property { function } mix
 * @property { function } unmix
 * @property { function } dtmf
 * @property { function } echo
 * @property { function } play
 * @property { function } record
 * @property { function } direction
 * @property { object } local
 * @property { number } local.port
 * @property { number } local.ssrc
 * @property { object } local.dtls
 * @property { string } local.dtls.fingerprint
 * @property { boolean } local.dtls.enabled
 * @property { string } local.dtls.icepwd
 */

/**
 * @function openchannel
 * @summary Opens a channel and returns a channel object.
 * @param { Object } [ properties ]
 * @param { string } [ properties.id ] Unique id provided which is simply returned in the channel object
 * @param { Object } [ properties.remote ]
 * @param { number } properties.remote.port - the remote port - must be an Int and should be even
 * @param { string } properties.remote.address - the remote (remote) host address
 * @param { number } properties.remote.codec - the remote codec as a number
 * @param { Object } [ properties.remote.dtls ]
 * @param { string } properties.remote.dtls.fingerprint - the fingerprint we verify the remote against
 * @param { string } properties.remote.dtls.setup - "active" or "passive"
 * @param { Object } [ properties.direction ] - direction from our perspective
 * @param { boolean } [ properties.direction.send = true ]
 * @param { boolean } [ properties.direction.recv = true ]
 * @param { channelcallback } [ callback ] - events are passed back to the caller via this callback
 * @returns { Promise< channel > } - the newly created channel
 */


/**
 * @typedef { function } codecfunc
 * @param { number } in - single val to input
 * @returns { number } - out
 */


/**
 * @typedef { object } tonegenerate
 * @param { string } tone - tone description
 * @param { string } file - wav file to append the data to, it will create new if it doesn't exist.
 * @returns { boolean }
 * @summary Generate tone from definition string
 * @description
We need to be able to generate tones. This is following the [standard](https://www.itu.int/dms_pub/itu-t/opb/sp/T-SP-E.180-2010-PDF-E.pdf).
Looping will be handled by soundsoup. Our generated file only needs to handle one cycle of the tone.

Our goal is to be efficient, so we do not generate this on the fly - most tones will be generated into wav files and
played when required.

If we want to play a tone continuously we should find a nicely looped file (e.g 1S will mean all frequencies in the
file will hit zero at the end of the file). This would simplify our generation.

In the standard we have definitions such as:

### United Kingdom of Great Britain and Northern Ireland
 - Busy tone - 400 0.375 on 0.375 off
 - Congestion tone - 400 0.4 on 0.35 off 0.225 on 0.525 off
 - Dial tone - 50//350+440 continuous
 - Number unobtainable tone - 400 continuous
 - Pay tone - 400 0.125 on 0.125 off
 - Payphone recognition tone - 1200/800 0.2 on 0.2 off 0.2 on 2.0 off
 - Ringing tone - 400+450//400x25//400x16 2/3 0.4 on 0.2 off 0.4 on 2.0 off

i.e. Tone - Frequency - Cadence

Frequency in Hz
 - f1×f2 f1 is modulated by f2
 - f1+f2 the juxtaposition of two frequencies f1 and f2 without modulation
 - f1/f2 f1 is followed by f2
 - f1//f2 in some exchanges frequency f1 is used and in others frequency f2 is used.
 - Cadence in seconds: ON – OFF

Try to keep our definitions as close to the standard. We also have to introduce some other items:

 - Amplitude
 - Change (in frequency or amplitude) - frequency can be handled by modulated

Take ringing tone:

400+450//400x25//400x16 2/3 0.4 *on 0.2 off 0.4 on 2.0 off*

We can ignore the // in our definition as we can simply choose the most common one.
So either 400+450 or 400x25
*Three does not appear ot be anything in the standard relating to the 2/3?*

Amplitude can be introduced by *
so

400+450 becomes 400+450*0.75 (every frequency will have its amplitude reduced).
400x25*0.75 is then also suported.

Increasing tones such as:
950/1400/1800

Cadence
950/1400/1800/0:333/333/333/1000
Note, we have introduced a final /0 to indicate silence. The cadences will iterated through for every / in the frequency list and is in mS (the standard lists in seconds). We don't need to support loops as soundsoup supports loops.
For:
950/1400/1800/0:333
Means each section will be 333mS.

Change
400+450*0.75~0 will reduce the amplitude from 0.75 to 0 during that cadence period
400~450 will increase the frequency during that cadence period

Note 400+450x300 is not supported.

UK Examples:

 - 350+440*0.5:1000 Dial tone
 - 400+450*0.5/0/400+450*0.5/0:400/200/400/2000 Ringing
 - 697+1209*0.5/0/697+1336*0.5/0/697+1477*0.5/0/697+1633*0.5/0:400/100 DTMF 123A
 - 770+1209*0.5/0/770+1336*0.5/0/770+1477*0.5/0/770+1633*0.5/0:400/100 DTMF 456B
 - 852+1209*0.5/0/852+1336*0.5/0/852+1477*0.5/0/852+1633*0.5/0:400/100 DTMF 789C
 - 941+1209*0.5/0/941+1336*0.5/0/941+1477*0.5/0/941+1633*0.5/0:400/100 DTMF *0#D
 - 440:1000 Unobtainable
 - 440/0:375/375 Busy
 - 440/0:400/350/225/525 Congestion
 - 440/0:125/125 Pay

```
tone.generate( "350+440*0.5:1000", "uksounds.wav" )
tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "uksounds.wav" )
...
```
A sound soup can then be used to index the times within the wav file.
 */

/**
 * @typedef { object } codecx
 * @property { codecfunc } linear162pcma
 * @property { codecfunc } pcma2linear16
 * @property { codecfunc } linear162pcmu
 * @property { codecfunc } pcmu2linear16
 */

/**
 * @summary functions for tone manipulation.
 * @memberof projectrtp
 * @property { tonegenerate } generate
 */

/**
 * @typedef { Object } wavinfo
 * @property { number } audioformat
 * @property { number } channelcount
 * @property { number } samplerate
 * @property { number } byterate
 * @property { number } bitdepth
 * @property { number } chunksize
 * @property { number } fmtchunksize
 * @property { number } subchunksize
 */

/**
 * @typedef { function } soundfileinfo
 * @param { string } filename
 * @returns { wavinfo }
 */

/**
 * @summary Loads wav header and returns info
 * @typedef { Object } soundfile
 * @property { function } info
 */

let actualprojectrtp
/**
 * Main interface to projectrtp. Provides access to either proxy nodes of projectrtp - or a local instance.
 */
class projectrtp {

  /** @type { codecx } */
  codecx

  /** @type { tonegenerate } */
  tone

  /** @type { proxy } */
  proxy

  /** @type { node.interface } */
  node

  /** @type { server.interface } */
  server

  /**
   * @hideconstructor
   */
  constructor() {
    /*
      Expose our node and server interface. node is the rtp node, server is the control server (i.e. sip)
     */
    this.node = node.interface.create( this )
    this.server = server.interface.create()

    /* to be retired - it is now confusing on direction to maintain in one interface */
    this.proxy = new proxy( this.node, this.server )
  }

  /**
   * Stsrtas our underlying server waiting for instructions. Must be called to initialise.
   * @param { object|undefined } params 
   * @returns { void }
   */
  run( params ) {

    if( "win32" == process.platform && "x64" == process.arch ) {
      throw new Error( "Platform not currently supported" )
    } else if( "win32" == process.platform && "ia32" == process.arch ) {
      throw new Error( "Platform not currently supported" )
    }

    if( actualprojectrtp ) return

    gencerts()
    if ( !params ) params = {}

    actualprojectrtp = require( bin )
    actualprojectrtp.run( params )
    this.dtls = actualprojectrtp.dtls
    this.tone = actualprojectrtp.tone
    this.rtpfilter = actualprojectrtp.rtpfilter
    this.codecx = actualprojectrtp.codecx
    this.soundfile = actualprojectrtp.soundfile
    this.rtpbuffer = actualprojectrtp.rtpbuffer
    this.stats = actualprojectrtp.stats
    this.shutdown = actualprojectrtp.shutdown
  }

  /**
   * Open a local channel (an RTP endpoint). Pass in params to confiure local and remote details.
   * @param { object } params 
   * @param { channelcallback } cb 
   */
  async openchannel( params = undefined, cb = undefined ) {
    if( "function" == typeof params ) {
      cb = params
      params = {}
    }

    if( "undefined" == typeof params ) params = {}

    if( !params.forcelocal && server.interface.get() ) {
      return server.interface.get().openchannel( params, cb )
    } else {
      const chan = actualprojectrtp.openchannel( params, ( d ) => {
        try{
          if( chan.em ) {
            chan.em.emit( "all", d )
            if( d.action ) chan.em.emit( d.action, d )
          }
        } catch ( e ) {
          console.trace( e )
        }
      } )
      /* I can't find a way of defining a getter in napi - so here we override */

      chan.local.address = localaddress
      chan.local.privateaddress = privateaddress

      chan.em = new EventEmitter()
      if( cb ) chan.em.on( "all", cb )

      if( params.id ) chan.id = params.id
      else chan.id = uuidv4()

      chan.uuid = uuidv4()

      /* ensure we are identicle to the node version of this object */
      chan.openchannel = this.openchannel.bind( this )

      return chan
    }
  }

  /**
   * Configure the local address we report back in any call to openchanel.
   * @param { string } address
   * @returns { void } 
   */
  setaddress( address ) {
    localaddress = address
  }

  /**
   * Configure our private address we report back in any call to openchanel.
   * @param { string } address
   * @returns { void }
   */
  setprivateaddress( address ) {
    privateaddress = address
  }
}

module.exports.projectrtp = new projectrtp()
