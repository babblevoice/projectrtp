
const fs = require( "fs" )

const projectrtp = require( "../index.js" ).projectrtp
const utils = require( "./utils.js" )

let scenarios = []
scenarios.push( require( "./mix3.scenario.js" ) )
scenarios.push( require( "./dtls.scenario.js" ) )
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

const maxnumberofsessions = 200
const secondsruntime = 3600*12
const minmscalllength = 50
const maxmscalllength = 1000 * 60 * 10 /* 1000 mS per second 60 seconds per minute , n minutes */

const rununtil = Math.floor( Date.now() / 1000 ) + secondsruntime

projectrtp.run()

const run = async () => {

  /* generate some useful files */
  await fs.promises.unlink( "/tmp/ukringing.wav" ).catch( ()=>{} )
  await fs.promises.unlink( "/tmp/powerdetectprofile.wav" ).catch( ()=>{} )
  projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "/tmp/ukringing.wav" )
  projectrtp.tone.generate( "0/800*0.5:700/2000", "/tmp/powerdetectprofile.wav" )

  while ( rununtil > Math.floor( Date.now() / 1000 ) ) {
    if( utils.currentchannelcount() < maxnumberofsessions ) {
      scenarios[ utils.between( 0, scenarios.length ) ]( utils.between( minmscalllength, maxmscalllength ) )
    }

    await utils.waitbetween( 0, 1000 )
  }

  /* It is safe to call shutdown before we are complete - it will just shutdown after all the work is done */
  projectrtp.shutdown()
}

run()
