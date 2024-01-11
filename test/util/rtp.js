


/**
 * Limitation of not parsing ccrc.
 * @param { Buffer } packet
 * @return { object }
 */
module.exports.parsepk = function( packet ) {
  return {
    sn: packet.readUInt16BE( 2 ),
    ts: packet.readUInt32BE( 4 ),
    pt: packet.readUInt8( 1 ) & 0x7f,
    ssrc: packet.readUInt32BE( 8 ),
    payload: new Uint8Array( packet.slice( 12 ) )
  }
}