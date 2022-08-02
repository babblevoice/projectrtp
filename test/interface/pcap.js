
const fs = require( "node:fs" )

function toHex( d ) {
  return ( "0" + ( Number( d ).toString( 16 ) ) ).slice( -2 ).toUpperCase()
}


module.exports.readpcap = async ( file, maxnumberofpackets = 1000 ) => {

  /* Buffer */
  const data = fs.readFileSync( file )
  if( !data ) return

  var fileposition = 0

  var ts_sec = 0
  var ts_usec = 0
  var ts_firstether = -1
  var frame = 0
  var ipv4hosts = []
  var etherframes = []


  // Do we need version info for now?
  //var uint16array = new Uint16Array(data)
  /* Magic number */
  if ( 2712847316 == data.readUInt32LE( 0 ) ) {
    /* Native byte order */
    console.log( "Native byte order" )
  } else if ( 3569595041 == data.readUInt32LE( 0 ) ) {
    /* Swapped byte order */
    console.log( "Swapped byte order" )
  } else if ( 2712812621 == data.readUInt32LE( 0 ) ) {
    /* Native byte order nano second timing */
    console.log( "Native byte order nano second timing" )
  } else if ( 1295823521 == data.readUInt32LE( 0 ) ) {
    /* Swapped byte order nano second timing */
    console.log( "Swapped byte order nano second timing" )
  }

  /* http://www.tcpdump.org/linktypes.html */
  if ( 1 != data.readUInt32LE( 5 * 4 ) ) {
    console.error( "Link layer should be LINKTYPE_ETHERNET", uint32array[ 5 ] )
    return
  }
  //console.log( "LINKTYPE_ETHERNET" )
  /* Read our first packet header */
  fileposition += 24

  for( let i = 0; i < maxnumberofpackets; i ++ ) {

    /* read packet header */
    ts_sec = data.readUInt32LE( fileposition )
    ts_usec = data.readUInt32LE( 4 + fileposition )
    var incl_len = data.readUInt32LE( 12 + fileposition )
    //var orig_len
    fileposition += 16

    if ( 0 == incl_len ) {
      continue
    }

    var etherpacket = {}
    etherpacket.frame = frame
    frame++
    etherpacket.ts_sec = ts_sec + ( ts_usec / 1000000 )
    if ( -1 == ts_firstether ) {
      ts_firstether = etherpacket.ts_sec
    }

    etherpacket.ts_sec_offset = ( ts_sec + ( ts_usec / 1000000 ) ) - ts_firstether
    //etherpacket.ts_usec = ts_usec
    etherpacket.src = "" + toHex( data.readUInt8( fileposition ) ) + ":" + toHex( data.readUInt8( fileposition + 1 ) ) + ":" + toHex( data.readUInt8( fileposition + 2 ) ) + ":" + toHex( data.readUInt8( fileposition + 3 ) ) + ":" + toHex( data.readUInt8( fileposition + 4 ) ) + ":" + toHex( data.readUInt8( fileposition + 5 ) )
    etherpacket.dst = "" + toHex( data.readUInt8( fileposition + 6 ) ) + ":" + toHex( data.readUInt8( fileposition + 7 ) ) + ":" + toHex( data.readUInt8( fileposition + 8 ) ) + ":" + toHex( data.readUInt8( fileposition + 9 ) ) + ":" + toHex( data.readUInt8( fileposition + 10 ) ) + ":" + toHex( data.readUInt8( fileposition + 11 ) )
    etherpacket.ethertype = "" + toHex( data.readUInt8( fileposition + 12 ) ) + toHex( data.readUInt8( fileposition + 13 ) )
    if ( parseInt( etherpacket.ethertype, 16 ) > 1536 ) {
      // Ref: https://en.wikipedia.org/wiki/EtherType
      switch ( etherpacket.ethertype ) {
      case "0800":
        /* IPV4 */
        etherpacket.ipv4 = {}
        etherpacket.ipv4.data = data.subarray( fileposition + 14, fileposition + 14 + incl_len )//uint8array.slice( 14, uint8array.length )
        etherpacket.ipv4.version = parseInt( toHex( ( etherpacket.ipv4.data[ 0 ] >> 4 ) & 0xf ), 16 )
        etherpacket.ipv4.ihl = parseInt( toHex( etherpacket.ipv4.data[ 0 ] & 0xf ), 16 )
        etherpacket.ipv4.dscp = toHex( ( etherpacket.ipv4.data[ 1 ] >> 2 ) & 0x3f )
        etherpacket.ipv4.ecn = toHex( etherpacket.ipv4.data[ 1 ] & 0x3 )
        etherpacket.ipv4.totallength = parseInt( toHex( etherpacket.ipv4.data[ 2 ] ) + toHex( etherpacket.ipv4.data[ 3 ] ), 16 )
        etherpacket.ipv4.identification = parseInt( toHex( etherpacket.ipv4.data[ 4 ] ) + toHex( etherpacket.ipv4.data[ 5 ] ), 16 )
        etherpacket.ipv4.flags = toHex( ( etherpacket.ipv4.data[ 6 ] >> 5 ) & 7 )
        etherpacket.ipv4.fragmentoffset = "" + toHex( etherpacket.ipv4.data[ 6 ] & 0x1f ) + toHex( etherpacket.ipv4.data[ 7 ] )
        etherpacket.ipv4.ttl = etherpacket.ipv4.data[ 8 ]
        etherpacket.ipv4.protocol = etherpacket.ipv4.data[ 9 ]
        etherpacket.ipv4.checksum = "" + toHex( etherpacket.ipv4.data[ 10 ] ) + toHex( etherpacket.ipv4.data[ 11 ] )
        etherpacket.ipv4.src = "" + etherpacket.ipv4.data[ 12 ] + "." + etherpacket.ipv4.data[ 13 ] + "." + etherpacket.ipv4.data[ 14 ] + "." + etherpacket.ipv4.data[ 15 ]
        etherpacket.ipv4.dst = "" + etherpacket.ipv4.data[ 16 ] + "." + etherpacket.ipv4.data[ 17 ] + "." + etherpacket.ipv4.data[ 18 ] + "." + etherpacket.ipv4.data[ 19 ]
        var hostid = -1
        if ( -1 == ( hostid = ipv4hosts.indexOf( etherpacket.ipv4.src ) ) ) {
          etherpacket.ipv4.srchostid = ipv4hosts.length
          ipv4hosts.push( etherpacket.ipv4.src )
        } else {
          etherpacket.ipv4.srchostid = hostid
        }

        if ( -1 == ( hostid = ipv4hosts.indexOf( etherpacket.ipv4.dst ) ) ) {
          etherpacket.ipv4.dsthostid = ipv4hosts.length
          ipv4hosts.push( etherpacket.ipv4.dst )
        } else {
          etherpacket.ipv4.dsthostid = hostid
        }

        switch ( etherpacket.ipv4.protocol ) {
        case 17:
          /* UDP */
          etherpacket.ipv4.udp = {}
          etherpacket.ipv4.udp.srcport = parseInt( toHex( etherpacket.ipv4.data[ 20 ] ) + toHex( etherpacket.ipv4.data[ 21 ] ), 16 )
          etherpacket.ipv4.udp.dstport = parseInt( toHex( etherpacket.ipv4.data[ 22 ] ) + toHex( etherpacket.ipv4.data[ 23 ] ), 16 )
          etherpacket.ipv4.udp.length = parseInt( toHex( etherpacket.ipv4.data[ 24 ] ) + toHex( etherpacket.ipv4.data[ 25 ] ), 16 )
          etherpacket.ipv4.udp.checksum = parseInt( toHex( etherpacket.ipv4.data[ 26 ] ) + toHex( etherpacket.ipv4.data[ 27 ] ), 16 )
          etherpacket.ipv4.udp.data = etherpacket.ipv4.data.subarray( 28, 28 + etherpacket.ipv4.data.length )
          break
        case 6:
          /* TCP */
          etherpacket.ipv4.tcp = {}
          etherpacket.ipv4.tcp.srcport = parseInt( toHex( etherpacket.ipv4.data[ 20 ] ) + toHex( etherpacket.ipv4.data[ 21 ] ), 16 )
          etherpacket.ipv4.tcp.dstport = parseInt( toHex( etherpacket.ipv4.data[ 22 ] ) + toHex( etherpacket.ipv4.data[ 23 ] ), 16 )
          etherpacket.ipv4.tcp.sequencenumber = parseInt( toHex( etherpacket.ipv4.data[ 24 ] ) + toHex( etherpacket.ipv4.data[ 25 ] ) + toHex( etherpacket.ipv4.data[ 26 ] ) + toHex( etherpacket.ipv4.data[ 27 ] ), 16 )
          etherpacket.ipv4.tcp.acknowledgmentnumber = parseInt( toHex( etherpacket.ipv4.data[ 28 ] ) + toHex( etherpacket.ipv4.data[ 29 ] ) + toHex( etherpacket.ipv4.data[ 30 ] ) + toHex( etherpacket.ipv4.data[ 31 ] ), 16 )
          etherpacket.ipv4.tcp.dataoffset = ( etherpacket.ipv4.data[ 32 ] >> 4 ) & 0xf
          etherpacket.ipv4.tcp.flags = {}
          etherpacket.ipv4.tcp.flags.ns = etherpacket.ipv4.data[ 32 ] & 1
          etherpacket.ipv4.tcp.flags.cwr = ( etherpacket.ipv4.data[ 33 ] >> 7 ) & 1
          etherpacket.ipv4.tcp.flags.ece = ( etherpacket.ipv4.data[ 33 ] >> 6 ) & 1
          etherpacket.ipv4.tcp.flags.urg = ( etherpacket.ipv4.data[ 33 ] >> 5 ) & 1
          etherpacket.ipv4.tcp.flags.ack = ( etherpacket.ipv4.data[ 33 ] >> 4 ) & 1
          etherpacket.ipv4.tcp.flags.psh = ( etherpacket.ipv4.data[ 33 ] >> 3 ) & 1
          etherpacket.ipv4.tcp.flags.rst = ( etherpacket.ipv4.data[ 33 ] >> 2 ) & 1
          etherpacket.ipv4.tcp.flags.syn = ( etherpacket.ipv4.data[ 33 ] >> 1 ) & 1
          etherpacket.ipv4.tcp.flags.fin = etherpacket.ipv4.data[ 33 ] & 1
          etherpacket.ipv4.tcp.windowsize = parseInt( toHex( etherpacket.ipv4.data[ 34 ] ) + toHex( etherpacket.ipv4.data[ 35 ] ), 16 )
          etherpacket.ipv4.tcp.checksum = parseInt( toHex( etherpacket.ipv4.data[ 36 ] ) + toHex( etherpacket.ipv4.data[ 37 ] ), 16 )
          etherpacket.ipv4.tcp.urgentpointer = parseInt( toHex( etherpacket.ipv4.data[ 38 ] ) + toHex( etherpacket.ipv4.data[ 39 ] ), 16 )
          etherpacket.ipv4.tcp.data = etherpacket.ipv4.data.subarray( 20 + ( etherpacket.ipv4.tcp.dataoffset * 4 ), 20 + ( etherpacket.ipv4.tcp.dataoffset * 4 ) + etherpacket.ipv4.data.length )
          break
        }
        break
      case "86DD":
        /* IPV6 */
        break
      case "0806":
        /* ARP */
        break
      case "9100":
        /* VLAN tagged */
        break
      }
    } else {
      // We probbaly won't need this as is raw length.
    }
    etherframes.push( etherpacket )
    fileposition += incl_len

    if( fileposition >= data.length ) break
  }

  return etherframes

}