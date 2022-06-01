
const fs = require( "fs" )

const projectrtp = require( "../index.js" ).projectrtp
const utils = require( "./utils.js" )

let scenarios = []
scenarios.push( require( "./echorecord.scenario.js" ) )
scenarios.push( require( "./playbackrecord.scenario.js" ) )
scenarios.push( require( "./echodualrecordpower.scenario.js" ) )
scenarios.push( require( "./echodualrecordpausestop.scenario.js" ) )
scenarios.push( require( "./mix2.scenario.js" ) )
scenarios.push( require( "./mixunmix.scenario.js" ) )

/*
The purpose of this script is to load up projectrtp to expose any issues with timing.
I am not worrying about checking the accuracy of calls etc - this is what our test
folder is for.

We have a problem with "we should never get here - we have no more buffer available on port"
*/

const maxnumberofsessions = 300
const secondsruntime = 3600*12
const minpkcalllength = 50
const maxpkcalllength = ( 50 * 60 ) * 5 /* 50 pks per second, 60 Sec per Min, n minutes */

const rununtil = Math.floor( Date.now() / 1000 ) + secondsruntime

projectrtp.run()

const run = async () => {

  /* generate some useful files */
  await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/uksounds.wav", ( err ) => { resolve() } ) } )
  projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "/tmp/ukringing.wav" )

  while ( rununtil > Math.floor( Date.now() / 1000 ) ) {
    if( utils.currentchannelcount() < maxnumberofsessions ) {
      scenarios[ utils.between( 0, scenarios.length ) ]( utils.between( minpkcalllength, maxpkcalllength ) )
    }

    await utils.waitbetween( 0, 1000 )
  }

  /* It is safe to call shutdown before we are complete - it will just shutdown after all the work is done */
  projectrtp.shutdown()
}

run()
