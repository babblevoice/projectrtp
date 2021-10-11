

#include <iostream>

#define _USE_MATH_DEFINES
#include <math.h>

#include <string>
#include <boost/algorithm/string.hpp>

#include "projectrtpsoundfile.h"
#include "projectrtptonegen.h"

typedef std::vector< std::string > vectorofstrings;

static void gentone( int16_t *outbuffer, int sizeofblock, double startfrequency, double endfrequency, double startamp, double endamp, int samplerate )
{
  if( 0 == startfrequency && 0 == endfrequency ) return;

  double angle;
  double ampatpos = startamp;
  double amppersample = ( endamp - startamp ) / sizeofblock;
  double freqpersample = ( endfrequency - startfrequency ) / sizeofblock;
  double frequencyatpos = startfrequency;
  if( 0 == startfrequency )
  {
    angle = 0;
  }
  else
  {
    angle = ( 2 * M_PI ) / ( double ) samplerate * startfrequency;
  }

  for( int i = 0; i < sizeofblock; i++ )
  {
    *outbuffer += static_cast< int16_t >( ( sin( angle * i ) * SHRT_MAX * ampatpos ) );
    outbuffer++;

    ampatpos += amppersample;
    frequencyatpos += freqpersample;

    if( 0 == frequencyatpos )
    {
      angle = 0;
    }
    else
    {
      angle = ( 2 * M_PI ) / ( double ) samplerate * frequencyatpos;
    }
  }
}


static void gen( std::string tone, std::string filename )
{
  vectorofstrings freqscadence;
  boost::split( freqscadence, tone, boost::is_any_of( ":" ) );

  if( 2 != freqscadence.size() )
  {
    std::cerr << "You must supply a cadence, eg. 400:1000" << std::endl;
    return;
  }

  vectorofstrings frequencies;
  vectorofstrings cadences;
  boost::split( frequencies, freqscadence[ 0 ], boost::is_any_of( "/" ) );
  boost::split( cadences, freqscadence[ 1 ], boost::is_any_of( "/" ) );

  uint8_t *readbuffer = nullptr;
  wavheader outwavheader;
  initwav( &outwavheader );

  /* 1. Calculate total time */
  vectorofstrings::iterator it;
  vectorofstrings cadenceparts;
  int cadencepos = 0;
  int cadencetotal = 0;
  for( it = frequencies.begin(); it != frequencies.end(); it++ )
  {
    cadencetotal += std::atoi( cadences[ cadencepos ].c_str() );
    cadencepos = ( cadencepos + 1 ) % cadences.size();
  }

  //std::cout << "Total time is " << cadencetotal << "mS" << std::endl;

  /* 2. Allocate enough memory */
  outwavheader.subchunksize = cadencetotal * outwavheader.sample_rate / 1000 * 2 /* 2 bytes per sample */;
  outwavheader.chunksize = outwavheader.subchunksize + 36;

  readbuffer = new uint8_t[ outwavheader.chunksize ];
  memset( readbuffer, 0, outwavheader.chunksize );

  /* 3. Generate tones */
  int pos = 0;
  cadencepos = 0;
  for( it = frequencies.begin(); it != frequencies.end(); it++ )
  {
    /* Current cadence. */
    int cadence = std::atoi( cadences[ cadencepos ].c_str() );
    cadencepos = ( cadencepos + 1 ) % cadences.size();

    int16_t *outbuffer = ( int16_t * ) &readbuffer[ pos * 2 ];
    int sizeofblock = outwavheader.sample_rate * cadence / 1000 /*mS*/;

    /* current frequency */
    /* *it could be 400+450 or or 400x25 450+450*0.75 or 450~480 or 450+450*0.75~1 */
    vectorofstrings freqamp;
    boost::split( freqamp, *it, boost::is_any_of( "*" ) );
    double startamp = 1;
    double endamp = 1;
    if( freqamp.size() > 1 )
    {
      vectorofstrings ampfromto;
      boost::split( ampfromto, freqamp[ 1 ], boost::is_any_of( "~" ) );
      startamp = std::atof( ampfromto[ 0 ].c_str() );
      endamp = startamp;
      if( ampfromto.size() > 1 )
      {
        endamp = std::atof( ampfromto[ 1 ].c_str() );
      }
    }
    vectorofstrings freqparts;
    boost::split( freqparts, freqamp[ 0 ], boost::is_any_of( "+x~" ) );

    double startfreq = -1;
    double endfreq = -1;

    std::size_t found = freqamp[ 0 ].find_first_of( "+x~" );
    if( std::string::npos == found )
    {
      startfreq = std::atof( freqparts[ 0 ].c_str() );
      gentone( outbuffer, sizeofblock, startfreq, startfreq, startamp, endamp, outwavheader.sample_rate );
      goto continueloop;
    }
    else
    {
      switch( freqamp[ 0 ][ found ] )
      {
        case '+':
        {
          for( auto freqit = freqparts.begin(); freqit != freqparts.end(); freqit++ )
          {
            startfreq = std::atof( freqit->c_str() );
            gentone( outbuffer, sizeofblock, startfreq, startfreq, startamp, endamp, outwavheader.sample_rate );
          }
          break;
        }
        case '~':
        {
          startfreq = std::atof( freqparts[ 0 ].c_str() );
          endfreq = std::atof( freqparts[ 1 ].c_str() );
          gentone( outbuffer, sizeofblock, startfreq, endfreq, startamp, endamp, outwavheader.sample_rate );
          break;
        }
      }
    }

continueloop:
    pos += sizeofblock;
  }


  /* Write */
  int file = open( filename.c_str(), O_RDWR | O_CREAT, S_IRUSR | S_IWUSR );
  __off_t position = lseek( file, 0, SEEK_END );
  if( 0 == position )
  {
    write( file, &outwavheader, sizeof( wavheader ) );
    write( file, readbuffer, outwavheader.chunksize );
  }
  else
  {
    wavheader currentheader;
    lseek( file, 0, SEEK_SET );
    read( file, &currentheader, sizeof( wavheader ) );
    lseek( file, 0, SEEK_END );

    if( currentheader.audio_format == outwavheader.audio_format &&
          currentheader.sample_rate == outwavheader.sample_rate )
    {
      /* Ok, good enough! */
      currentheader.chunksize += outwavheader.chunksize;
      currentheader.subchunksize += outwavheader.subchunksize;

      lseek( file, 0, SEEK_SET );
      write( file, &currentheader, sizeof( wavheader ) );
      lseek( file, 0, SEEK_END );
      write( file, readbuffer, outwavheader.chunksize );
    }
    else
    {
      std::cerr << "File format to append should be the same" << std::endl;
    }
  }

  /* Clean up */
  if( nullptr != readbuffer )
  {
    delete[] readbuffer;
  }

  close( file );
}

#ifdef NODE_MODULE
/*
Our node functions.
*/
static napi_value createnapibool( napi_env env, bool v ) {
  napi_value result;
  napi_create_uint32( env, v == true? 1 : 0, &result );
  napi_coerce_to_bool( env, result, &result );
  return result;
}

static napi_value tonegen( napi_env env, napi_callback_info info ) {
  size_t argc = 2;
  napi_value argv[ 2 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 2 != argc ) {
    napi_throw_error( env, "0", "You must provide tone.generate( toneformat /*e.g. 350+440*0.5:1000*/, filename )" );
    return NULL;
  }

  size_t bytescopied;
  char tonedcescription[ 256 ];
  char filename[ 256 ];

  napi_get_value_string_utf8( env, argv[ 0 ], tonedcescription, sizeof( tonedcescription ), &bytescopied );
  if( 0 == bytescopied || bytescopied >= sizeof( tonedcescription ) ) {
    napi_throw_error( env, "1", "Tone definition bad or too long" );
    return NULL;
  }

  napi_get_value_string_utf8( env, argv[ 1 ], filename, sizeof( filename ), &bytescopied );
  if( 0 == bytescopied || bytescopied >= sizeof( filename ) ) {
    napi_throw_error( env, "1", "Filename too long" );
    return NULL;
  }

  gen( tonedcescription, filename );

  return createnapibool( env, true );
}

void inittonegen( napi_env env, napi_value &result ) {
  napi_value tonegenobj;
  napi_value nfunction;

  if( napi_ok != napi_create_object( env, &tonegenobj ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "tone", tonegenobj ) ) return;
  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, tonegen, nullptr, &nfunction ) ) return;
  if( napi_ok != napi_set_named_property( env, tonegenobj, "generate", nfunction ) ) return;

}
#endif
