

#include <iostream>
#include "projectrtpsoundsoup.h"


soundsoupfile::pointer soundsoupfile::create() {
  return pointer( new soundsoupfile() );
}

soundsoupfile::soundsoupfile() :
  start( 0 ),
  stop( -1 ),
  loopcount( -1 ),
  maxloop( -1 ),
  sf( nullptr ) {

}

soundsoupfile::~soundsoupfile() {
  this->sf = nullptr;
}

/*
# c'stor
*/
soundsoup::soundsoup( size_t size ) :
  loopcount( -1 ),
  currentfile( 0 ) {

  this->files.resize( size );
}

/*
# d'stor
*/
soundsoup::~soundsoup() {

}

/*
# create
Shared pointer version of us.
*/
soundsoup::pointer soundsoup::create( size_t size ) {
  return pointer( new soundsoup( size ) );
}

void soundsoup::plusone( soundsoupfile::pointer playing ) {
  /* Do we loop this file? */
  if( 0 != playing->loopcount ) {
    playing->sf->setposition( playing->start );
    playing->loopcount--;
    return;
  }

  /* We have played the last file */
  if( this->files.size() == this->currentfile + 1 ) {
    if( 0 == this->loopcount ) return;
    this->loopcount--;

    for( auto it = this->files.begin(); it != this->files.end(); it++ ) {
      (*it)->loopcount = (*it)->maxloop;
      if( (*it)->sf ) {
        (*it)->sf->setposition( (*it)->start );
      }
    }
    this->currentfile = 0;
    return;
  }

  this->currentfile++;
}

bool soundsoup::read( rawsound &out ) {
  if( 0 == this->files.size() ) {
    return false;
  }

  soundsoupfile::pointer playing = this->files[ this->currentfile ];

  if( !playing->sf ) {
    this->plusone( playing );
    return false;
  } else if ( playing->sf->complete() ) {
    this->plusone( playing );
    return false;
  } else if ( -1 != playing->stop && playing->sf->getposition() > playing->stop ) {
    this->plusone( playing );
    return false;
  }

  playing->sf->read( out );
  return true;
}

void soundsoup::addfile( soundsoupfile::pointer p, int index ) {
  this->files[ index ] = p;
}

static std::string getfilenamefromobjectforcodec(
                        napi_env env,
                        napi_value obj,
                        const char* first,
                        const char* second,
                        const char* third ) {

  bool result;
  size_t bytescopied;
  char buf[ 256 ];
  napi_value nwav;

  if( napi_ok == napi_has_named_property( env, obj, first, &result ) && result ) {
    if( napi_ok == napi_get_named_property( env, obj, first, &nwav ) ) {
      napi_get_value_string_utf8( env, nwav, buf, sizeof( buf ), &bytescopied );
      return std::string( buf );
    }
  }

  if( napi_ok == napi_has_named_property( env, obj, second, &result ) && result ) {
    if( napi_ok == napi_get_named_property( env, obj, second, &nwav ) ) {
      napi_get_value_string_utf8( env, nwav, buf, sizeof( buf ), &bytescopied );
      return std::string( buf );
    }
  }

  if( napi_ok == napi_has_named_property( env, obj, third, &result ) && result ) {
    if( napi_ok == napi_get_named_property( env, obj, third, &nwav ) ) {
      napi_get_value_string_utf8( env, nwav, buf, sizeof( buf ), &bytescopied );
      return std::string( buf );
    }
  }

  return buf;
}

static soundsoupfile::pointer parsefileobj( soundsoup::pointer p, napi_env env, napi_value obj, int format ) {

  soundsoupfile::pointer ssf = soundsoupfile::create();

  napi_value nloop;

  if( napi_ok == napi_get_named_property( env, obj, "loop", &nloop ) ) {

    bool loop = false;
    if( napi_ok == napi_get_value_bool( env, nloop, &loop ) && loop ) {
      ssf->loopcount = INT_MAX;
    }

    uint32_t loopcount;
    if( napi_ok == napi_get_value_uint32( env, nloop, &loopcount ) ) {
      ssf->loopcount = loopcount;
    }
  }

  std::string filename;
  switch( format ) {
    case PCMUPAYLOADTYPE: {
      filename = getfilenamefromobjectforcodec( env, obj, "pcmu", "l168k", "wav" );
      break;
    }
    case PCMAPAYLOADTYPE: {
      filename = getfilenamefromobjectforcodec( env, obj, "pcma", "l168k", "wav" );
      break;
    }

    case G722PAYLOADTYPE: {
      filename = getfilenamefromobjectforcodec( env, obj, "g722", "l1616k", "wav" );
      break;
    }

    case ILBCPAYLOADTYPE: {
      filename = getfilenamefromobjectforcodec( env, obj, "ilbc", "l168k", "wav" );
      break;
    }

    default: {
      filename = getfilenamefromobjectforcodec( env, obj, "l168k", "l1616k", "wav" );
      break;
    }
  }

  ssf->sf = soundfile::create( filename );

  if( !ssf->sf->isopen() ) {
    return nullptr;
  }

  return ssf;
}

soundsoup::pointer soundsoupcreate( napi_env env, napi_value obj, int channelcodec ) {
  soundsoup::pointer p = nullptr;
  napi_value nfiles;

  bool loop = false;
  napi_get_value_bool( env, obj, &loop );

  if( napi_ok == napi_get_named_property( env, obj, "files", &nfiles ) ) {
    bool isarray;
    napi_is_array( env, nfiles, &isarray );

    if( !isarray ) {
      return nullptr;
    }

    uint32_t numfiles;
    napi_get_array_length( env, nfiles, &numfiles );

    if( 0 == numfiles ) {
      return nullptr;
    }

    p = soundsoup::create( numfiles );

    for( uint32_t i = 0; i < numfiles; i ++ ) {
      napi_value filei;
      napi_get_element( env, nfiles, i, &filei );
      soundsoupfile::pointer ssp = parsefileobj( p, env, filei, channelcodec );
      if( nullptr == ssp ) return nullptr;
      p->addfile( ssp, i );
    }

    return p;
  }

  return nullptr;
}
