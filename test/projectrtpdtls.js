/*
Currently we use ourselelves to test us working - which is not ideal but better than nothing.
*/
const expect = require( "chai" ).expect
const projectrtp = require( "../index.js" ).projectrtp

/* Tests */
describe( "dtls", function() {

  it( `Test we have a fingerprint global`, async function() {
    expect( projectrtp.dtls.fingerprint ).to.be.a( "string" )
    expect( projectrtp.dtls.fingerprint.length ).to.equal( 95 )
  } )

  it( `Test we have a fingerprint in channel`, async function() {

    let channel = projectrtp.openchannel( { "target": { "address": "localhost", "port": 1000, "codec": 0 } }, function( d ) {
    } )

    expect( channel.local.dtls.fingerprint ).to.be.a( "string" )
    expect( channel.local.dtls.fingerprint.length ).to.equal( 95 )

    expect( channel.local.dtls.enabled ).to.be.a( "boolean" )
    expect( channel.local.dtls.enabled ).to.equal( false )

    channel.close()
  } )

  it( `Create 2 channels and negotiate`, async function() {
/*
    let channela = projectrtp.openchannel( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
    } )

    let channelb = projectrtp.openchannel( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
    } )
*/
  } )
} )
