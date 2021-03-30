
#include <iostream>
#include <stdio.h>
#include <signal.h>

#include <sys/resource.h>

#include <boost/asio.hpp>
#include <boost/bind/bind.hpp>
#include <boost/shared_ptr.hpp>
#include <boost/enable_shared_from_this.hpp>
#include <boost/lexical_cast.hpp>
#include <unordered_map>

#include <atomic>
#include <thread>

#include <chrono>

#include "controlclient.h"
#include "json.hpp"
#include "projectdaemon.h"
#include "projectrtpchannel.h"
#include "projectrtppacket.h"
#include "projectrtpsoundfile.h"
#include "projectrtptonegen.h"
#include "projectrtpcodecx.h"

#include "firfilter.h"

boost::asio::io_context iocontext;
boost::asio::io_context workercontext;

std::string mediachroot;

typedef boost::asio::basic_waitable_timer< std::chrono::high_resolution_clock > ourhighrestimer;
ourhighrestimer periodictimer( workercontext );

rtpchannels dormantchannels;

std::string publicaddress;
std::string controlhost;
unsigned maxworker;
std::atomic_bool running;


/*!md
## ontimer
Generally atm a keepalive timer.
*/
void ontimer(const boost::system::error_code& /*e*/)
{
  periodictimer.expires_at( periodictimer.expiry() + std::chrono::seconds( 20 ) );
  periodictimer.async_wait( &ontimer );
}


/*!md
# stopserver
Actually do the stopping
*/
static void stopserver( void )
{
  running = false;
  iocontext.stop();
  workercontext.stop();
}

/*!md
# killServer
As it says...
*/
static void killserver( int signum )
{
  std::cout << "OUCH" << std::endl;
  stopserver();
}

/*!md
## workerthread
Worker threads perform transcoding and generally the workload of mixing etc.
*/
void workerthread( void )
{
  while( running )
  {
    try
    {
      workercontext.run();
    }
    catch( std::exception& e )
    {
      std::cerr << e.what() << std::endl;
    }
    catch( ... )
    {
      std::cerr << "Unhandled exception in worker bees - rentering workercontext" << std::endl;
    }
    std::cout << "Worker bee finished" << std::endl;
  }
}

/*!md
# startserver
Start our server and kick off all of the worker threads.

Try strategy of 1 thread per core. Work should be shared out between them. We have to be careful about data access whilst trying not to use Mutex for performance.
To create a worker thread item:

Note for sometime:
ioservice->post( boost::bind( somefunction, shared_from_this() ) );

*/
void startserver()
{
  running = true;
  unsigned numcpus = std::thread::hardware_concurrency();

  if( 0 != maxworker && numcpus > maxworker )
  {
    numcpus = maxworker;
  }

  numcpus--;
  if( 0 == numcpus )
  {
    numcpus = 1;
  }

  std::cout << "Starting " << numcpus << " worker threads" << std::endl;

  // A mutex ensures orderly access to std::cout from multiple threads.
  std::mutex iomutex;
  std::vector< std::thread > threads( numcpus );

  periodictimer.expires_at( std::chrono::system_clock::now() );
  periodictimer.async_wait( &ontimer );

  try
  {
    controlclient::pointer c = controlclient::create( iocontext, controlhost );

    cpu_set_t cpuset;
    CPU_ZERO( &cpuset );
    CPU_SET( 0, &cpuset );

    if ( pthread_setaffinity_np( pthread_self(), sizeof( cpu_set_t ), &cpuset ) )
    {
      std::cerr << "Error tying thread to CPU " << 0 << std::endl;
    }

    for ( unsigned i = 0; i < numcpus; i++ )
    {
      threads[ i ] = std::thread( workerthread );

      CPU_ZERO( &cpuset );
      CPU_SET( ( i + 1 ) % numcpus, &cpuset );

      if ( pthread_setaffinity_np( threads[ i ].native_handle() , sizeof( cpu_set_t ), &cpuset ) )
      {
        std::cerr << "Error tying thread to CPU " << 0 << std::endl;
      }
    }

    while( running )
    {
      try
      {
        iocontext.run();
      }
      catch( std::exception& e )
      {
        std::cerr << e.what() << std::endl;
      }
      catch( ... )
      {
        std::cerr << "Unhandled exception in iocontext" << std::endl;
      }
    }

    std::cout << "Waiting for threads to join" << std::endl;
    for ( auto& t : threads )
    {
      t.join();
    }
  }
  catch( std::exception& e )
  {
    std::cerr << e.what() << std::endl;
  }

  // Clean up
  std::cout << "Cleaning up" << std::endl;
  return;
}


/*!md
# initchannels
Create our channel objects and pre allocate any memory.
*/
void initchannels( unsigned short startport, unsigned short endport )
{
  int i;

  try
  {
    // Test we can open them all if needed and warn if necessary.
    std::string dummycontrol;
    for( i = startport; i < endport; i += 2 )
    {
      projectrtpchannel::pointer p = projectrtpchannel::create( workercontext, i );

      std::string id, uuid;
      p->open( id, uuid, controlclient::pointer( nullptr ) );
      dormantchannels.push_back( p );
    }
  }
  catch(...)
  {
    std::cerr << "I could only open " << dormantchannels.size() << " channels, you might want to review your OS settings." << std::endl;
  }

  // Now close.
  rtpchannels::iterator it;
  for( it = dormantchannels.begin(); it != dormantchannels.end(); it++ )
  {
    // This closes the channel in this thread
    (* it )->doclose();
  }
}

/*!md
# testatomic
Test an atomic variable to ensure it is lock free. Issue a warning if it is not as we rely on atomic variables for performance with threads.
*/
void testatomic( void )
{
  std::atomic_bool test;
  if( !test.is_lock_free() )
  {
    std::cerr << "Warning: atomic variables appear to be not atomic so we will be using locks which will impact performance" << std::endl;
  }
}

/*!md
# main
As it says...
*/
int main( int argc, const char* argv[] )
{
  publicaddress = "127.0.0.1";
  controlhost = "127.0.0.1";
  maxworker = 0;

  unsigned short startrtpport = 10000;
  unsigned short endrtpport = 20000;

  srand( time( NULL ) );

  bool fg = false;

  for ( int i = 1; i < argc ; i++ )
  {
    if ( argv[i] != NULL )
    {
      std::string argvstr = argv[ i ];

      if( "--help" == argvstr )
      {
        std::cout << "--fg - do not daemonize tune the program in the foreground." << std::endl;
        std::cout << "--connect - the host to connect to receive control messages from" << std::endl;
        std::cout << "--pa - the public address we tell the client to send RTP to." << std::endl;
        std::cout << "--maxworker - we launch a thread per core, if you wish to change this use this." << std::endl;
        std::cout << "--chroot - setup the root directory for where soundfiles are read and written to." << std::endl;
        std::cout << "--tone tone file.wav - output tone generation to a wave file";
        exit( 0 );
      }
      else if ( "--fg" == argvstr )
      {
        fg = true;
      }
      else if( "--connect" == argvstr )
      {
        try
        {
          if( argc > ( i + 1 ) )
          {
            controlhost = argv[ i + 1 ];
            i++;
            continue;
          }
        }
        catch( boost::bad_lexical_cast &e )
        {
        }
        std::cerr << "Need more info (control address)..." << std::endl;
        return -1;
      }
      else if( "--pa" == argvstr ) /* [P]ublic RTP [A]ddress */
      {
        try
        {
          if( argc > ( i + 1 ) )
          {
            publicaddress = argv[ i + 1 ];
            i++;
            continue;
          }
        }
        catch( boost::bad_lexical_cast &e )
        {
        }
        std::cerr << "Need more info..." << std::endl;
        return -1;
      }
      else if( "--maxworker" == argvstr )
      {
        try
        {
          if( argc > ( i + 1 ) )
          {
            maxworker = boost::lexical_cast< int >( argv[ i + 1 ] );
            i++;
            continue;
          }
        }
        catch( boost::bad_lexical_cast &e )
        {
        }
        std::cerr << "I need a maxworker count" << std::endl;
        return -1;
      }
      else if( "--testfir" == argvstr )
      {
        if( argc <= ( i + 1 ) )
        {
          std::cerr << "I need a frequency to generate (in Hz) to pass into our LP filter - quiting " << std::endl;
          return -1;
        }
        std::cout << i << ":" << argc << std::endl;
        int frequency = boost::lexical_cast< int >( argv[ i + 1 ] );
        testlofir( frequency );
        testma();
        return 0;
      }
      else if( "--chroot" == argvstr )
      {
        i++;
        mediachroot = argv[ i ];
        if( '/' != mediachroot[ mediachroot.size() -1 ] )
        {
          mediachroot += "/";
        }
      }
      else if( "--wavinfo" == argvstr )
      {
        i++;
        wavinfo( argv[ i ] );
        return 0;
      }
      else if( "--tone" == argvstr )
      {
        gentone( argv[ i + 1 ], argv[ i + 2 ] );
        return 0;
      }
      else if( "--test" == argvstr )
      {
        /* run tests */
        codectests();
        return 0;
      }
    }
  }

  // Register our CTRL-C handler
  signal( SIGINT, killserver );
  std::cout << "Starting Project RTP server" << std::endl;
  std::cout << "RTP published address is " << publicaddress << std::endl;
  std::cout << "RTP ports "  << startrtpport << " => " << endrtpport << ": " << (int) ( ( endrtpport - startrtpport ) / 2 ) << " channels" << std::endl;
  if( 0 != maxworker )
  {
    std::cout << "Max worker threads " << maxworker << std::endl;
  }

  testatomic();
  initchannels( startrtpport, endrtpport );

  /* init transcoding stuff */
  gen711convertdata();

  if ( !fg )
  {
    daemonize();
  }

  std::cout << "Started RTP server, waiting for requests." << std::endl;

  startserver();

  return 0;
}
