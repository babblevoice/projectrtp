
const { v4: uuidv4 } = require( "uuid" )
const EventEmitter = require( "events" )
const { Readable, Writable } = require( "stream" )

const server = require( "./lib/server.js" )
const node = require( "./lib/node.js" )

const fs = require( "fs" )

let localaddress = "127.0.0.1"
let privateaddress = "127.0.0.1"

let bin = "./build/Release/projectrtp"
if ( fs.existsSync( "./build/Debug/projectrtp.node" ) ) {
  console.log( "Dev build exists - using dev build..." )
  bin = "./build/Debug/projectrtp"
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

  /**
   * 
   * @param {*} nodes 
   */
  setnodes( nodes ) {
    this._server.setnodes( nodes )
  }

  /**
   * 
   */
  getnodes() {
    return this._server.getnodes()
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
 * @property { function } playrecord
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
      /* Build chan.local from the Rust napi-class getters (port/ssrc/icepwd/
         dtlsfingerprint). napi-rs class getters can't be replaced on the
         instance, so we attach `local` as a regular own-property here. */
      Object.defineProperty( chan, "local", {
        value: {
          port: chan.port,
          ssrc: chan.ssrc,
          icepwd: chan.icepwd,
          address: localaddress,
          privateaddress: privateaddress,
          dtls: {
            fingerprint: chan.dtlsfingerprint,
            enabled: false,
            icepwd: chan.icepwd,
          },
        },
        writable: true,
        configurable: true,
        enumerable: true,
      } )

      chan.em = new EventEmitter()
      if( cb ) chan.em.on( "all", cb )

      if( params.id ) chan.id = params.id
      else chan.id = uuidv4()

      chan.uuid = uuidv4()

      /* ensure we are identicle to the node version of this object */
      chan.openchannel = this.openchannel.bind( this )

      /* Wrap the Rust `createReadStream(opts, cb)` napi method so JS gets a
         standard Node `Readable` — pipeable into fs.createWriteStream,
         WebSocket, fetch bodies, any transform stream, etc.

         The underlying napi method fires its callback once per 20 ms frame
         with a Buffer; a zero-length Buffer is the end-of-stream sentinel
         (fired by the Rust forwarder task when the channel closes or the
         reader is explicitly destroyed).

         Guarded: defensively skip the wrapping if the native module
         doesn't expose these methods (older builds), so this shim stays
         loadable. */
      if( "function" !== typeof chan.createReadStream ) return chan
      const napiCreateReadStream = chan.createReadStream.bind( chan )
      const napiDestroyReadStream = chan.destroyReadStream.bind( chan )
      chan.createReadStream = function( opts = {} ) {
        /* Resolve defaults here so the Readable can publish what it is —
           sample rate and format don't change for the life of the reader,
           so the consumer reads these properties once at setup rather
           than pulling metadata off every frame (which would break `pipe`). */
        const resolved = {
          direction:   opts.direction   || "in",
          format:      opts.format      || "l16",
          samplerate:  ( opts.samplerate === 16000 ) ? 16000 : 8000,
          numchannels: ( 2 === opts.numchannels ) ? 2 : 1,
        }

        const stream = new Readable( { read() { /* push-driven */ } } )
        let destroyed = false

        const id = napiCreateReadStream( opts, ( buf ) => {
          if( destroyed ) return
          if( 0 === buf.length ) {
            /* end-of-stream sentinel from the Rust forwarder */
            destroyed = true
            stream.push( null )
            return
          }
          /* Backpressure: if the Readable's internal buffer is full
             (consumer slow), `push` returns false — drop the frame to
             keep memory bounded. Mirrors the Rust-side mpsc drop policy. */
          stream.push( buf )
        } )

        if( !id ) {
          /* Channel wasn't accepting commands — defer-destroy the stream
             with an error so pipe consumers unwind cleanly. */
          setImmediate( () => stream.destroy( new Error( "createReadStream: channel not accepting" ) ) )
          return stream
        }

        /* Publish the resolved config so consumers (STT client, WAV
           encoder, websocket sender, etc.) know the byte shape they're
           seeing without needing per-frame metadata. Frozen to
           discourage consumers mutating and expecting any effect. */
        stream.format      = resolved.format
        stream.samplerate  = resolved.samplerate
        stream.numchannels = resolved.numchannels
        stream.direction   = resolved.direction
        stream.readerId    = id

        stream._destroy = function( err, cb ) {
          if( !destroyed ) {
            destroyed = true
            napiDestroyReadStream( id )
            stream.push( null )
          }
          cb( err )
        }

        return stream
      }

      /* Mirror of createReadStream on the write side: Node `Writable`
         whose bytes flow through a bounded Rust-side mpsc and are
         emitted onto the channel's outbound leg as 20 ms frames. v1
         format is locked to linear PCM-16 LE at 8 kHz mono. */
      const napiCreateWriteStream = chan.createWriteStream.bind( chan )
      const napiPushWriterBytes = chan.pushWriterBytes.bind( chan )
      const napiEndWriteStream = chan.endWriteStream.bind( chan )
      const napiDestroyWriteStream = chan.destroyWriteStream.bind( chan )
      chan.createWriteStream = function( opts = {} ) {
        const resolved = {
          format:      "l16",
          samplerate:  8000,
          numchannels: 1,
        }

        const id = napiCreateWriteStream( opts )
        if( !id ) {
          const s = new Writable( { write( _c, _e, cb ) { cb( new Error( "createWriteStream: channel not accepting" ) ) } } )
          setImmediate( () => s.destroy( new Error( "createWriteStream: channel not accepting" ) ) )
          return s
        }

        /* The Writable's internal highWaterMark gives us a natural
           backpressure signal: if `_write`'s cb() is delayed, the
           caller's next `.write()` returns false and they await 'drain'.
           When the Rust-side 1 s queue is full we delay cb() with a
           short setImmediate until the tick drains. This keeps cb()
           non-blocking while still applying backpressure upstream. */
        const stream = new Writable( {
          highWaterMark: 16 * 1024, // 16 KB ≈ 500 ms of L16 @ 8 kHz
          write( chunk, _enc, cb ) {
            const push = () => {
              if( napiPushWriterBytes( id, chunk ) ) return cb()
              /* Rust buffer is full — retry after the Node event loop
                 yields. The tick fires every 20 ms so one retry is
                 usually enough. Scheduler-friendly backoff; no busy loop. */
              setImmediate( push )
            }
            push()
          },
          final( cb ) {
            napiEndWriteStream( id )
            cb()
          },
          destroy( err, cb ) {
            napiDestroyWriteStream( id )
            cb( err )
          }
        } )

        stream.format      = resolved.format
        stream.samplerate  = resolved.samplerate
        stream.numchannels = resolved.numchannels
        stream.writerId    = id

        return stream
      }

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
