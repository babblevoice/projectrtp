

const projectrtp = require( "../../index.js" ).projectrtp
const dgram = require( "dgram" )
const crypto = require( "crypto" )
const { expect } = require( "chai" )

before( function() {
  projectrtp.run()
} )

/**
 * Build a STUN Binding Request with USERNAME, PRIORITY, USE-CANDIDATE,
 * ICE-CONTROLLING, MESSAGE-INTEGRITY and FINGERPRINT attributes.
 * @param { string } localufrag - the server's ice-ufrag
 * @param { string } remoteufrag - the client's ice-ufrag
 * @param { string } key - the server's ice-pwd (used for MESSAGE-INTEGRITY)
 * @returns { Buffer }
 */
function buildstunrequest( localufrag, remoteufrag, key ) {

  const transid = crypto.randomBytes( 12 )

  /* USERNAME attribute (0x0006) = "localufrag:remoteufrag" */
  const username = Buffer.from( `${localufrag}:${remoteufrag}` )
  const usernamepadded = Buffer.alloc( Math.ceil( username.length / 4 ) * 4 )
  username.copy( usernamepadded )
  const usernameattr = Buffer.alloc( 4 + usernamepadded.length )
  usernameattr.writeUInt16BE( 0x0006, 0 )
  usernameattr.writeUInt16BE( username.length, 2 )
  usernamepadded.copy( usernameattr, 4 )

  /* PRIORITY attribute (0x0024) */
  const priorityattr = Buffer.alloc( 8 )
  priorityattr.writeUInt16BE( 0x0024, 0 )
  priorityattr.writeUInt16BE( 4, 2 )
  priorityattr.writeUInt32BE( 0x6e001eff, 4 )

  /* USE-CANDIDATE attribute (0x0025) - empty */
  const usecandidateattr = Buffer.alloc( 4 )
  usecandidateattr.writeUInt16BE( 0x0025, 0 )
  usecandidateattr.writeUInt16BE( 0, 2 )

  /* ICE-CONTROLLING attribute (0x802a) */
  const icecontrollingattr = Buffer.alloc( 12 )
  icecontrollingattr.writeUInt16BE( 0x802a, 0 )
  icecontrollingattr.writeUInt16BE( 8, 2 )
  crypto.randomBytes( 8 ).copy( icecontrollingattr, 4 )

  const attrs = Buffer.concat( [ usernameattr, priorityattr, usecandidateattr, icecontrollingattr ] )

  /* build header: type=0x0001 (Binding Request), length=attrs + integrity(24) + fingerprint(8) */
  const header = Buffer.alloc( 20 )
  header.writeUInt16BE( 0x0001, 0 )
  header.writeUInt16BE( attrs.length + 24 + 8, 2 )
  header.writeUInt32BE( 0x2112a442, 4 )
  transid.copy( header, 8 )

  /* MESSAGE-INTEGRITY (0x0008): HMAC-SHA1 over header + attrs, with length adjusted */
  const preintegrity = Buffer.concat( [ header, attrs ] )
  /* set length to include up to end of integrity attr */
  preintegrity.writeUInt16BE( attrs.length + 24, 2 )
  const hmac = crypto.createHmac( "sha1", key ).update( preintegrity ).digest()
  const integrityattr = Buffer.alloc( 24 )
  integrityattr.writeUInt16BE( 0x0008, 0 )
  integrityattr.writeUInt16BE( 20, 2 )
  hmac.copy( integrityattr, 4 )

  /* restore full length for fingerprint */
  preintegrity.writeUInt16BE( attrs.length + 24 + 8, 2 )

  /* FINGERPRINT (0x8028): CRC32 XOR 0x5354554e */
  const prefingerprint = Buffer.concat( [ preintegrity, integrityattr ] )
  const crc = crc32( prefingerprint ) ^ 0x5354554e
  const fingerprintattr = Buffer.alloc( 8 )
  fingerprintattr.writeUInt16BE( 0x8028, 0 )
  fingerprintattr.writeUInt16BE( 4, 2 )
  fingerprintattr.writeUInt32BE( crc >>> 0, 4 )

  return Buffer.concat( [ prefingerprint, fingerprintattr ] )
}

/**
 * Standard CRC32 (same polynomial as used by STUN FINGERPRINT)
 */
function crc32( buf ) {
  let crc = 0xffffffff
  for( let i = 0; i < buf.length; i++ ) {
    crc = ( crc >>> 8 ) ^ crc32table[ ( crc ^ buf[ i ] ) & 0xff ]
  }
  return ( crc ^ 0xffffffff ) >>> 0
}

const crc32table = ( function() {
  const table = new Uint32Array( 256 )
  for( let i = 0; i < 256; i++ ) {
    let c = i
    for( let j = 0; j < 8; j++ ) {
      c = ( c & 1 ) ? ( 0xedb88320 ^ ( c >>> 1 ) ) : ( c >>> 1 )
    }
    table[ i ] = c >>> 0
  }
  return table
} )()

/**
 * Parse a STUN Binding Success Response and extract the XOR-MAPPED-ADDRESS
 * @param { Buffer } pkt
 * @returns {{ port: number, address: string } | false }
 */
function parsestunresponse( pkt ) {
  if( pkt.length < 20 ) return false
  const msgtype = pkt.readUInt16BE( 0 )
  if( msgtype !== 0x0101 ) return false /* not a binding success */
  const cookie = pkt.readUInt32BE( 4 )
  if( cookie !== 0x2112a442 ) return false

  let offset = 20
  const end = 20 + pkt.readUInt16BE( 2 )
  while( offset + 4 <= end ) {
    const attrtype = pkt.readUInt16BE( offset )
    const attrlen = pkt.readUInt16BE( offset + 2 )
    if( attrtype === 0x0020 ) { /* XOR-MAPPED-ADDRESS */
      const family = pkt.readUInt8( offset + 5 )
      if( family === 0x01 ) {
        const xport = pkt.readUInt16BE( offset + 6 ) ^ ( 0x2112a442 >>> 16 )
        const xaddr = pkt.readUInt32BE( offset + 8 ) ^ 0x2112a442
        const a = ( xaddr >>> 24 ) & 0xff
        const b = ( xaddr >>> 16 ) & 0xff
        const c = ( xaddr >>> 8 ) & 0xff
        const d = xaddr & 0xff
        return { port: xport, address: `${a}.${b}.${c}.${d}` }
      }
    }
    const padded = attrlen + ( ( 4 - ( attrlen % 4 ) ) % 4 )
    offset += 4 + padded
  }
  return false
}


describe( "STUN ICE timing", function() {

  it( "multiple rapid STUN requests all get individual responses", async function() {

    this.timeout( 5000 )
    this.slow( 3000 )

    const projecticepwd = "testpasswordfortesting12"
    const clientufrag = "cUfr"
    const serverufrag = "sUfr"

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channel = await projectrtp.openchannel( {
      "id": "10",
      "remote": {
        "address": "localhost",
        "port": 20000,
        "codec": 0,
        "icepwd": "clientpasswordhere123456"
      },
      "local": {
        "icepwd": projecticepwd,

      }
    }, ( d ) => {
      if( "close" === d.action ) done( d )
    } )

    const server = dgram.createSocket( "udp4" )

    const responses = []
    const sendtimes = []
    const requestcount = 5

    server.on( "message", function( msg ) {
      const parsed = parsestunresponse( msg )
      responses.push( { time: process.hrtime.bigint(), parsed } )

      if( responses.length === requestcount ) {
        server.close()
        channel.close()
      }
    } )

    server.bind()
    server.on( "listening", function() {

      /* send 5 STUN requests rapidly */
      for( let i = 0; i < requestcount; i++ ) {
        const pkt = buildstunrequest( serverufrag, clientufrag, projecticepwd )
        sendtimes.push( process.hrtime.bigint() )
        server.send( pkt, channel.local.port, "localhost" )
      }
    } )

    const closedata = await finished

    expect( responses.length ).to.equal( requestcount )

    /* each response should be a valid binding success */
    for( const r of responses ) {
      expect( r.parsed ).to.not.equal( false )
      expect( r.parsed.address ).to.equal( "127.0.0.1" )
    }

    /* all responses should arrive within 100ms of the first send */
    const firstsend = sendtimes[ 0 ]
    const lastresponse = responses[ responses.length - 1 ].time
    const totalms = Number( lastresponse - firstsend ) / 1e6
    expect( totalms ).to.be.below( 100 )
  } )


  it( "STUN requests interleaved with DTLS-like packet still get responses", async function() {

    this.timeout( 5000 )
    this.slow( 3000 )

    const projecticepwd = "anothertestpassword12345"
    const clientufrag = "dUfr"
    const serverufrag = "eUfr"

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channel = await projectrtp.openchannel( {
      "id": "11",
      "remote": {
        "address": "localhost",
        "port": 20000,
        "codec": 0,
        "icepwd": "clientpwd2here1234567890"
      },
      "local": {
        "icepwd": projecticepwd,

      }
    }, ( d ) => {
      if( "close" === d.action ) done( d )
    } )

    const server = dgram.createSocket( "udp4" )

    const stunresponses = []
    const sendtimes = []

    server.on( "message", function( msg ) {
      const parsed = parsestunresponse( msg )
      if( parsed ) {
        stunresponses.push( { time: process.hrtime.bigint(), parsed } )
      }

      /* 2 STUN before DTLS + 2 STUN after DTLS = 4 total */
      if( stunresponses.length === 4 ) {
        server.close()
        channel.close()
      }
    } )

    server.bind()
    server.on( "listening", function() {

      /* send 2 STUN requests */
      for( let i = 0; i < 2; i++ ) {
        const pkt = buildstunrequest( serverufrag, clientufrag, projecticepwd )
        sendtimes.push( process.hrtime.bigint() )
        server.send( pkt, channel.local.port, "localhost" )
      }

      /* send a DTLS-like packet (content type 20-63, first byte 0x16 = DTLS handshake) */
      /* this should be buffered as early DTLS since no DTLS session exists yet */
      const fakeDtls = Buffer.alloc( 50 )
      fakeDtls[ 0 ] = 0x16 /* DTLS content type: handshake */
      server.send( fakeDtls, channel.local.port, "localhost" )

      /* send 2 more STUN requests after the DTLS packet */
      for( let i = 0; i < 2; i++ ) {
        const pkt = buildstunrequest( serverufrag, clientufrag, projecticepwd )
        sendtimes.push( process.hrtime.bigint() )
        server.send( pkt, channel.local.port, "localhost" )
      }
    } )

    const closedata = await finished

    expect( stunresponses.length ).to.equal( 4 )

    /* all STUN responses should be valid */
    for( const r of stunresponses ) {
      expect( r.parsed ).to.not.equal( false )
      expect( r.parsed.address ).to.equal( "127.0.0.1" )
    }

    /* all responses should arrive promptly - within 100ms of first send */
    const firstsend = sendtimes[ 0 ]
    const lastresponse = stunresponses[ stunresponses.length - 1 ].time
    const totalms = Number( lastresponse - firstsend ) / 1e6
    expect( totalms ).to.be.below( 100 )
  } )


  it( "each STUN response contains correct unique data (no buffer reuse corruption)", async function() {

    this.timeout( 5000 )
    this.slow( 3000 )

    const projecticepwd = "buffertestreusepwd123456"
    const clientufrag = "fUfr"
    const serverufrag = "gUfr"

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channel = await projectrtp.openchannel( {
      "id": "12",
      "remote": {
        "address": "localhost",
        "port": 20000,
        "codec": 0,
        "icepwd": "clientpwd3here1234567890"
      },
      "local": {
        "icepwd": projecticepwd,

      }
    }, ( d ) => {
      if( "close" === d.action ) done( d )
    } )

    const server = dgram.createSocket( "udp4" )

    /* send from different source ports to get different XOR-MAPPED-ADDRESS in responses */
    const servers = []
    const responses = []
    const requestcount = 3
    let closedcount = 0

    function trydone() {
      if( responses.length === requestcount ) {
        for( const s of servers ) s.close()
        channel.close()
      }
    }

    for( let i = 0; i < requestcount; i++ ) {
      const s = dgram.createSocket( "udp4" )
      servers.push( s )

      s.on( "message", function( msg ) {
        const parsed = parsestunresponse( msg )
        if( parsed ) {
          responses.push( { index: i, parsed, raw: Buffer.from( msg ) } )
          trydone()
        }
      } )

      s.bind( () => {
        const pkt = buildstunrequest( serverufrag, clientufrag, projecticepwd )
        s.send( pkt, channel.local.port, "localhost" )
      } )
    }

    const closedata = await finished

    expect( responses.length ).to.equal( requestcount )

    /* verify each response has the correct XOR-MAPPED-ADDRESS reflecting the sender's port */
    /* all should be from 127.0.0.1 but with different ports */
    const ports = responses.map( r => r.parsed.port )
    const uniqueports = new Set( ports )
    expect( uniqueports.size ).to.equal( requestcount )

    /* verify all transaction IDs in responses are different */
    const transids = responses.map( r => r.raw.slice( 8, 20 ).toString( "hex" ) )
    const uniquetransids = new Set( transids )
    expect( uniquetransids.size ).to.equal( requestcount )
  } )
} )
