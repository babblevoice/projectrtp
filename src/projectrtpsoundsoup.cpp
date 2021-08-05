

#include <iostream>

#include "projectrtpsoundsoup.h"


/*!md
# c'stor
*/
soundsoup::soundsoup( void ) :
  loopcount( -1 ),
  currentfile( 0 )
{
}

/*!md
# d'stor
*/
soundsoup::~soundsoup()
{

}

/*!md
# create
Shared pointer version of us.
*/
soundsoup::pointer soundsoup::create( void )
{
  return pointer( new soundsoup() );
}


/*!md
# getpreferredfilename
Return the preferred filename based on the format we are sending data to.
*/
std::string *soundsoup::getpreferredfilename( JSON::Object &file, int format )
{
  switch( format )
  {
    case PCMUPAYLOADTYPE:
    {
      if( file.has_key( "pcmu" ) )
      {
        return &JSON::as_string( file[ "pcmu" ] );
      }
      if( file.has_key( "pcma" ) )
      {
        return &JSON::as_string( file[ "pcma" ] );
      }
      if( file.has_key( "l168k" ) )
      {
        return &JSON::as_string( file[ "l168k" ] );
      }
      if( file.has_key( "l1616k" ) )
      {
        return &JSON::as_string( file[ "l1616k" ] );
      }
      if( file.has_key( "ilbc" ) )
      {
        return &JSON::as_string( file[ "ilbc" ] );
      }
      if( file.has_key( "g722" ) )
      {
        return &JSON::as_string( file[ "g722" ] );
      }
      if( file.has_key( "wav" ) )
      {
        return &JSON::as_string( file[ "wav" ] );
      }
      break;
    }
    case PCMAPAYLOADTYPE:
    {
      if( file.has_key( "pcma" ) )
      {
        return &JSON::as_string( file[ "pcma" ] );
      }
      if( file.has_key( "pcmu" ) )
      {
        return &JSON::as_string( file[ "pcmu" ] );
      }
      if( file.has_key( "l168k" ) )
      {
        return &JSON::as_string( file[ "l168k" ] );
      }
      if( file.has_key( "l1616k" ) )
      {
        return &JSON::as_string( file[ "l1616k" ] );
      }
      if( file.has_key( "ilbc" ) )
      {
        return &JSON::as_string( file[ "ilbc" ] );
      }
      if( file.has_key( "g722" ) )
      {
        return &JSON::as_string( file[ "g722" ] );
      }
      if( file.has_key( "wav" ) )
      {
        return &JSON::as_string( file[ "wav" ] );
      }
      break;
    }
    case G722PAYLOADTYPE:
    {
      if( file.has_key( "g722" ) )
      {
        return &JSON::as_string( file[ "g722" ] );
      }
      if( file.has_key( "l1616k" ) )
      {
        return &JSON::as_string( file[ "l1616k" ] );
      }
      if( file.has_key( "l168k" ) )
      {
        return &JSON::as_string( file[ "l168k" ] );
      }
      if( file.has_key( "pcma" ) )
      {
        return &JSON::as_string( file[ "pcma" ] );
      }
      if( file.has_key( "pcmu" ) )
      {
        return &JSON::as_string( file[ "pcmu" ] );
      }
      if( file.has_key( "ilbc" ) )
      {
        return &JSON::as_string( file[ "ilbc" ] );
      }
      if( file.has_key( "wav" ) )
      {
        return &JSON::as_string( file[ "wav" ] );
      }
      break;
    }
    case ILBCPAYLOADTYPE:
    {
      if( file.has_key( "ilbc" ) )
      {
        return &JSON::as_string( file[ "ilbc" ] );
      }
      if( file.has_key( "l168k" ) )
      {
        return &JSON::as_string( file[ "l168k" ] );
      }
      if( file.has_key( "pcma" ) )
      {
        return &JSON::as_string( file[ "pcma" ] );
      }
      if( file.has_key( "pcmu" ) )
      {
        return &JSON::as_string( file[ "pcmu" ] );
      }
      if( file.has_key( "l1616k" ) )
      {
        return &JSON::as_string( file[ "l1616k" ] );
      }
      if( file.has_key( "g722" ) )
      {
        return &JSON::as_string( file[ "g722" ] );
      }
      if( file.has_key( "wav" ) )
      {
        return &JSON::as_string( file[ "wav" ] );
      }
      break;
    }
  }

  return nullptr;
}

/*!md
# config
Config (or reconfig) this soup.
*/
void soundsoup::config( JSON::Object &json, int format )
{
  if( json.has_key( "files" ) )
  {
    /* This is the only reason for our existence! */
    JSON::Array rfiles = JSON::as_array( json[ "files" ] );
    this->files.resize( rfiles.values.size() );
    size_t num = 0;

    if( this->currentfile > this->files.size() )
    {
      this->currentfile = 0;
    }

    for( auto it = rfiles.values.begin(); it != rfiles.values.end(); it++ )
    {
      JSON::Object &inref = JSON::as_object( *it );
      soundsoupfile &ref = this->files[ num ];

      std::string *newfilename = this->getpreferredfilename( inref, format );

      /* recreate soundfile object if there was no sound or if the filename has changed */
      if( !ref.sf || *newfilename != ref.sf->geturl() )
      {
        this->currentfile = 0;
        if( newfilename )
        {
          ref.sf = soundfile::create( *newfilename );
          if( !ref.sf->isopen() )
          {
            std::cerr << "Problem with file: " << JSON::to_string( inref ) << std::endl;
          }
        }
        else
        {
          ref.sf = nullptr;
        }
      }

      /* Defaults */
      ref.loopcount = 0;
      ref.maxloop = 0;
      ref.start = 0;
      ref.stop = -1;

      if( inref.has_key( "loop" ) )
      {
        switch( inref[ "loop" ].which() )
        {
          case 2: /* bool */
          {
            if( JSON::as_boolean( inref[ "loop" ] ) == JSON::Bool( true ) )
            {
              ref.loopcount = INT_MAX;
            }
            break;
          }
          case 6: /* int */
          {
            ref.loopcount = JSON::as_int64( inref[ "loop" ] );
          }
        }
        ref.loopcount = JSON::as_int64( inref[ "loop" ] );
        ref.maxloop = ref.loopcount;
      }

      if( inref.has_key( "start" ) )
      {
        ref.start = JSON::as_int64( inref[ "start" ] );
        ref.sf->setposition( ref.start );
      }

      if( inref.has_key( "stop" ) )
      {
        ref.stop = JSON::as_int64( inref[ "stop" ] );
      }
      num++;
    }
  }

  this->loopcount = 0;
  if( json.has_key( "loop" ) )
  {
    switch( json[ "loop" ].which() )
    {
      case 2: /* bool */
      {
        if( JSON::as_boolean( json[ "loop" ] ) == JSON::Bool( true ) )
        {
          this->loopcount = INT_MAX;
        }
        break;
      }
      case 6: /* int */
      {
        this->loopcount = JSON::as_int64( json[ "loop" ] );
      }
    }
  }
}

void soundsoup::plusone( soundsoupfile &playing )
{
  /* Do we loop this file? */
  if( 0 != playing.loopcount )
  {
    playing.sf->setposition( playing.start );
    playing.loopcount--;
    return;
  }

  /* We have played the last file */
  if( this->files.size() == this->currentfile + 1 )
  {
    if( 0 == this->loopcount ) return;
    this->loopcount--;

    for( auto it = this->files.begin(); it != this->files.end(); it++ )
    {
      it->loopcount = it->maxloop;
      if( it->sf )
      {
        it->sf->setposition( it->start );
      }
    }
    this->currentfile = 0;
    return;
  }

  this->currentfile++;
}

bool soundsoup::read( rawsound &out )
{
  if( 0 == this->files.size() )
  {
    return false;
  }

  soundsoupfile &playing = this->files[ this->currentfile ];

  if( !playing.sf )
  {
    this->plusone( playing );
    return false;
  }
  else if ( playing.sf->complete() )
  {
    this->plusone( playing );
    return false;
  }
  else if ( -1 != playing.stop && playing.sf->getposition() > playing.stop )
  {
    this->plusone( playing );
    return false;
  }

  playing.sf->read( out );
  return true;
}

/*!md
# c'stor
*/
soundsoupfile::soundsoupfile() :
  start( 0 ),
  stop( -1 ),
  loopcount( -1 ),
  maxloop( -1 ),
  sf( nullptr )
{

}

/*!md
# d'stor
*/
soundsoupfile::~soundsoupfile()
{
  this->sf = nullptr;
}
