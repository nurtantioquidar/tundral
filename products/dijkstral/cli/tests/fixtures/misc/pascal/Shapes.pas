unit Shapes;

{$mode objfpc}

interface

uses
  SysUtils, Classes,
  Geometry in 'Geometry.pas';

type
  TColor = (clRed, clGreen);
  TShape = class;
  TShapeClass = class of TShape;

  IDrawable = interface(IInterface)
    procedure Paint;
  end;

  TShape = class(TBaseShape, IDrawable)
  private
    FName: string;
    procedure SetName(const Value: string);
  public
    constructor Create(const AName: string);
    procedure Paint;
    procedure Draw; overload;
    procedure Draw(Scale: Integer); overload;
    function Area: Double; virtual;
  end;

  TCircle = class(TShape)
    function Area: Double; override;
    procedure Grow;
  end;

procedure Register;
function Compute(A, B: Integer): Integer;

implementation

uses
  Math;

procedure Log(const Msg: string);
begin
  { implementation only }
end;

procedure Register;
begin
  Log('register');
end;

function Compute(A, B: Integer): Integer;
begin
  Result := A + B;
  Register;
end;

constructor TShape.Create(const AName: string);
begin
  inherited Create;
  SetName(AName);
end;

procedure TShape.SetName(const Value: string);
begin
  FName := Value;
end;

procedure TShape.Paint;
var
  Scale: Integer;
begin
  Scale := 2;
  Self.SetName('painted');
  Prepare;
end;

procedure TShape.Draw;
begin
end;

procedure TShape.Draw(Scale: Integer);
begin
end;

function TShape.Area: Double;
begin
  Result := 0;
end;

function TCircle.Area: Double;
begin
  Result := Compute(1, 2); Grow;
end;

procedure TCircle.Grow;
begin
  if True then Draw;
  Paint;
end;

end.
