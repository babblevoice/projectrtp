

#include <iostream>
#include "projectrtpsoundsoup.h"


soundsoupfile::pointer soundsoupfile::create() {
  return pointer( new soundsoupfile() );
}

soundsoupfile::soundsoupfile() :
  start( 0 ),
  stop( -1 ),
  loopcount( 0 ),
  maxloop( 0 ),
  sf( nullptr ) {

}

soundsoupfile::~soundsoupfile() {
  this->sf = nullptr;
}

/*
# c'stor
*/
soundsoup::soundsoup( size_t size ) :
  loopcount( 0 ),
  currentfile( 0 ),
  finished( false ),
  files() {

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

/*
Move onto the next item in the sound soup. If there is no more to play
return false otherwise return true.
*/
bool soundsoup::plusone( soundsoupfile::pointer playing ) {
  /* Do we loop this file? */
  if( playing->loopcount > 0 ) {
    playing->sf->setposition( playing->start );
    playing->loopcount--;
    return true;
  }

  /* Have we have played the last file */
  if( this->files.size() == this->currentfile + 1 ) {
    if( this->loopcount > 0 ) {
      this->loopcount--;

      for( auto it = this->files.begin(); it != this->files.end(); it++ ) {
        (*it)->loopcount = (*it)->maxloop;
        if( (*it)->sf ) {
          (*it)->sf->setposition( (*it)->start );
        }
      }
      this->currentfile = 0;

      return true;
    }
    this->finished = true;
    return false;
  }

  this->currentfile++;
  soundsoupfile::pointer next = this->files[ this->currentfile ];
  next->sf->setposition( next->start );

  return true;
}

bool soundsoup::read( rawsound &out ) {
  if( 0 == this->files.size() || this->finished ) {
    return false;
  }

  soundsoupfile::pointer playing = this->files[ this->currentfile ];
  playing->sf->read( out );

  if ( playing->sf->complete() ) {
    this->plusone( playing );
  } else if ( -1 != playing->stop && playing->sf->getposition() > playing->stop ) {
    this->plusone( playing );
  }

  return true;
}

void soundsoup::addfile( soundsoupfile::pointer p, int index ) {
  this->files[ index ] = p;
}

#ifdef NODE_MODULE
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

  bool hasit = false;
  if( napi_ok == napi_has_named_property( env, obj, "loop", &hasit ) && hasit ) {
    napi_value nloop;

    if( napi_ok == napi_get_named_property( env, obj, "loop", &nloop ) ) {
      napi_valuetype typeresult;
      napi_typeof( env, nloop, &typeresult );
      if( napi_boolean == typeresult ) {
        bool loop = false;
        napi_get_value_bool( env, nloop, &loop );
        if( loop ) {
          ssf->loopcount = INT_MAX;
          ssf->maxloop = ssf->loopcount;
        }
      } else {
        int32_t vloop;
        napi_get_value_int32( env, nloop, &vloop );
        if( vloop > 0 ) {
          ssf->loopcount = vloop - 1;
          ssf->maxloop = ssf->loopcount;
        }
      }
    }
  }

  if( napi_ok == napi_has_named_property( env, obj, "start", &hasit ) && hasit ) {
    napi_value nstart;

    if( napi_ok == napi_get_named_property( env, obj, "start", &nstart ) ) {
      int32_t vstart;
      napi_get_value_int32( env, nstart, &vstart );
      ssf->start = vstart;
    }
  }

  if( napi_ok == napi_has_named_property( env, obj, "stop", &hasit ) && hasit ) {
    napi_value nstop;

    if( napi_ok == napi_get_named_property( env, obj, "stop", &nstop ) ) {
      int32_t vstop;
      napi_get_value_int32( env, nstop, &vstop );
      ssf->stop = vstop;
    }
  }

  std::string filename;
  /* TODO - currently untested */
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

  ssf->sf = soundfilereader::create( filename );
  if( !ssf->sf->isopen() ) {
    return nullptr;
  }

  if( ssf->start > 0 ) {
    ssf->sf->setposition( ssf->start );
  }

  return ssf;
}

soundsoup::pointer soundsoupcreate( napi_env env, napi_value obj, int channelcodec ) {
  soundsoup::pointer p = nullptr;
  napi_value nfiles;

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


    bool hasit = false;
    if( napi_ok == napi_has_named_property( env, obj, "loop", &hasit ) && hasit ) {
      napi_value nloop;

      if( napi_ok == napi_get_named_property( env, obj, "loop", &nloop ) ) {
        napi_valuetype typeresult;
        napi_typeof( env, nloop, &typeresult );
        if( napi_boolean == typeresult ) {
          bool loop = false;
          napi_get_value_bool( env, nloop, &loop );
          if( loop ) {
            p->setloop( INT_MAX );
          }
        } else {
          int32_t vloop;
          napi_get_value_int32( env, nloop, &vloop );
          if( vloop > 0 ) p->setloop( vloop - 1 );
        }
      }
    }

    return p;
  }

  return nullptr;
}

#endif /* NODE_MODULE */
